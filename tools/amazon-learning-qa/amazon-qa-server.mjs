#!/usr/bin/env node

import { createServer } from "node:http";
import { mkdir, readFile, readdir, rename, unlink, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { execFile, spawn } from "node:child_process";
import path from "node:path";
import { createHash, randomUUID } from "node:crypto";
import { promisify } from "node:util";

import {
  buildDossierSessionSeed,
  buildDossierOverview,
  buildDossierWorkbench,
  buildProductIntake,
  buildLearningDossier,
  buildOpenHumanMemoryDocument,
  normalizeStoredDossier,
  validateLearningDossierForSave,
  updateDossierBusinessVerificationState,
  updateDossierEvidenceDecisionState,
  updateDossierExperimentResultState,
  updateDossierReviewState,
  updateDossierSelfTestState,
} from "./amazon-dossier-lib.mjs";
import {
  AMAZON_AUTHORS,
  USER_SOURCE_AUTHOR,
  buildKnowledgeHealthSummary,
  buildKnowledgeReadinessSummary,
  buildQaPayload,
  buildRetrievalQuery,
  buildWorkflowIntent,
  buildSourceContextFromArticle,
  DEFAULT_SUGGESTED_QUESTIONS,
  isAuthorComparisonRequest,
  normalizeContextText,
  parseOpenHumanContext,
  workflowIntentTemplate,
} from "./amazon-qa-lib.mjs";
import {
  buildSourceTreeDrainPreflight,
  buildSourceTreeCalibration,
  sourceTreeSearchTerms,
  summarizeSourceTreeDrain,
  summarizeSourceTreeStatus,
} from "./amazon-source-tree-lib.mjs";
import { resolveAmazonQaPaths } from "./amazon-qa-paths.mjs";

const QA_PATHS = resolveAmazonQaPaths(import.meta.dirname);
const ROOT = QA_PATHS.root;
const UI_PATH = QA_PATHS.uiPath;
const MANIFEST_PATH = QA_PATHS.manifestPath;
const DB_PATH = QA_PATHS.memoryDbPath;
const MEMORY_TREE_DB_PATH = QA_PATHS.memoryTreeDbPath;
const DOCS_DIR = QA_PATHS.docsDir;
const CORE_BIN = QA_PATHS.coreBin;
const SOURCE_TREE_DRAIN_RUNNER = QA_PATHS.sourceTreeDrainRunnerPath;
const WORKSPACE = QA_PATHS.workspace;
const DOSSIER_ROOT = path.join(WORKSPACE, "learning-archives");
const NOTEBOOK_ROOT = path.join(WORKSPACE, "learning-notebooks");
const USER_SOURCE_ROOT = path.join(WORKSPACE, "user-sources");
const LEARNING_NOTE_ROOT = path.join(WORKSPACE, "learning-notes");
const RUN_DIR = path.join(WORKSPACE, "run");
const SOURCE_TREE_DRAIN_STATE_PATH = path.join(RUN_DIR, "source-tree-drain.json");
const SOURCE_TREE_DRAIN_STOP_PATH = path.join(RUN_DIR, "source-tree-drain.stop");

const DEFAULT_HOST = "127.0.0.1";
const DEFAULT_PORT = 7790;
const DEFAULT_CORE_PORT = 7789;
const DEFAULT_NAMESPACE = "amazon-learning";
const DEFAULT_TOKEN = "openhuman-amazon-local-token";
const MAX_SESSION_MESSAGES = 64;
const NOTEBOOK_BOUNDARY = "学习专题会话保存的是用户问题、系统整理、来源引用和学习路径；它不是作者原文证据，不能混入 amazon-learning 作者资料库。";
const LEARNING_NOTE_BOUNDARY = "学习笔记保存的是用户整理或系统回答摘录；它不是作者原文证据，只有用户主动转成“我的资料”后才会作为用户资料参与问答。";
const TEST_NOTEBOOK_ID_PATTERNS = [/^amazon-qa-smoke-/i, /^feedback-check-/i];

let coreProcess = null;
let sourceTreeDrainProcess = null;
const sessions = new Map();
const dossierWriteQueues = new Map();
const execFileAsync = promisify(execFile);

function usage() {
  console.log(`Usage:
  node tools/amazon-qa-server.mjs [--host ${DEFAULT_HOST}] [--port ${DEFAULT_PORT}] [--core-port ${DEFAULT_CORE_PORT}] [--no-core]

Open the returned local URL in your browser.`);
}

function argValue(args, name, fallback = undefined) {
  const index = args.indexOf(name);
  if (index === -1) return fallback;
  const value = args[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`Missing value for ${name}`);
  return value;
}

async function main() {
  const args = process.argv.slice(2);
  if (args.includes("--help") || args.includes("-h")) {
    usage();
    return;
  }

  const host = argValue(args, "--host", DEFAULT_HOST);
  const port = Number(argValue(args, "--port", String(DEFAULT_PORT)));
  const corePort = Number(argValue(args, "--core-port", String(DEFAULT_CORE_PORT)));
  const noCore = args.includes("--no-core");
  const token = process.env.OPENHUMAN_CORE_TOKEN || DEFAULT_TOKEN;
  const namespace = process.env.OPENHUMAN_NAMESPACE || DEFAULT_NAMESPACE;
  const coreBaseUrl = `http://${DEFAULT_HOST}:${corePort}`;
  const rpcUrl = `${coreBaseUrl}/rpc`;

  if (!noCore) {
    await ensureCoreRunning({ coreBaseUrl, corePort, token });
  }

  const server = createServer(async (request, response) => {
    try {
      await routeRequest(request, response, { rpcUrl, token, namespace, coreBaseUrl, corePort });
    } catch (error) {
      const status = Number(error?.statusCode || error?.status || 500);
      sendJson(response, status >= 400 && status < 600 ? status : 500, { error: friendlyError(error) });
    }
  });

  server.listen(port, host, () => {
    console.log(`Amazon learning Q&A: http://${host}:${port}`);
    console.log(`Knowledge namespace: ${namespace}`);
  });

  const shutdown = () => {
    server.close(() => {});
    if (coreProcess && !coreProcess.killed) coreProcess.kill("SIGTERM");
  };

  process.on("SIGINT", () => {
    shutdown();
    process.exit(0);
  });
  process.on("SIGTERM", () => {
    shutdown();
    process.exit(0);
  });
}

async function routeRequest(request, response, context) {
  const url = new URL(request.url || "/", "http://localhost");

  if (request.method === "GET" && url.pathname === "/favicon.ico") {
    response.writeHead(204, { "cache-control": "no-store" });
    response.end();
    return;
  }

  if (request.method === "GET" && url.pathname === "/") {
    const html = await readFile(UI_PATH, "utf8");
    sendText(response, 200, html, "text/html; charset=utf-8");
    return;
  }

  if (request.method === "GET" && url.pathname === "/api/status") {
    sendJson(response, 200, await getStatus(context));
    return;
  }

  if (request.method === "GET" && url.pathname === "/api/source-tree-drain") {
    const status = await getStatus(context);
    sendJson(response, 200, { sourceTreeDrain: status.sourceTreeDrain, sourceTree: status.health?.sourceTree || {} });
    return;
  }

  if (request.method === "POST" && url.pathname === "/api/source-tree-drain/start") {
    const body = await readJsonBody(request);
    sendJson(response, 200, { sourceTreeDrain: await startSourceTreeDrain(context, body) });
    return;
  }

  if (request.method === "POST" && url.pathname === "/api/source-tree-drain/stop") {
    sendJson(response, 200, { sourceTreeDrain: await stopSourceTreeDrain(context) });
    return;
  }

  if (request.method === "POST" && url.pathname === "/api/source-context") {
    const body = await readJsonBody(request);
    sendJson(response, 200, { sourceContext: await buildSourceContextResponse(context, body) });
    return;
  }

  if (request.method === "POST" && url.pathname === "/api/source-selection") {
    const body = await readJsonBody(request);
    sendJson(response, 200, { sourceSelection: await buildQuestionSourceSelection(context, body) });
    return;
  }

  if (request.method === "GET" && url.pathname === "/api/user-sources") {
    sendJson(response, 200, { userSources: await listUserSources(context.namespace) });
    return;
  }

  if (request.method === "POST" && url.pathname === "/api/user-sources") {
    const body = await readJsonBody(request);
    sendJson(response, 201, { userSource: await saveUserSource(context.namespace, body) });
    return;
  }

  if (request.method === "DELETE" && url.pathname.startsWith("/api/user-sources/")) {
    const id = decodeURIComponent(url.pathname.slice("/api/user-sources/".length));
    await deleteUserSource(context.namespace, id);
    sendJson(response, 200, { ok: true });
    return;
  }

  if (request.method === "GET" && url.pathname === "/api/notes") {
    sendJson(response, 200, { notes: await listLearningNotes(context.namespace) });
    return;
  }

  if (request.method === "POST" && url.pathname === "/api/notes") {
    const body = await readJsonBody(request);
    sendJson(response, 201, { note: await saveLearningNoteRecord(context.namespace, body) });
    return;
  }

  const noteUserSourceMatch = url.pathname.match(/^\/api\/notes\/([^/]+)\/user-source$/);
  if (request.method === "POST" && noteUserSourceMatch) {
    const id = decodeURIComponent(noteUserSourceMatch[1]);
    sendJson(response, 201, { userSource: await convertLearningNoteToUserSource(context.namespace, id) });
    return;
  }

  if (request.method === "DELETE" && url.pathname.startsWith("/api/notes/")) {
    const id = decodeURIComponent(url.pathname.slice("/api/notes/".length));
    await deleteLearningNote(context.namespace, id);
    sendJson(response, 200, { ok: true });
    return;
  }

  if (request.method === "GET" && url.pathname === "/api/notebooks") {
    const notebookList = await listNotebooks(context.namespace);
    sendJson(response, 200, {
      notebooks: notebookList.notebooks,
      hiddenTestNotebookCount: notebookList.hiddenTestNotebookCount,
    });
    return;
  }

  const notebookStudyPackMatch = url.pathname.match(/^\/api\/notebooks\/([^/]+)\/study-pack$/);
  if (request.method === "GET" && notebookStudyPackMatch) {
    const id = decodeURIComponent(notebookStudyPackMatch[1]);
    sendJson(response, 200, { studyPack: buildNotebookStudyPack(await readNotebook(context.namespace, id)) });
    return;
  }

  const notebookAnswerEffectivenessMatch = url.pathname.match(/^\/api\/notebooks\/([^/]+)\/answer-effectiveness$/);
  if (request.method === "POST" && notebookAnswerEffectivenessMatch) {
    const id = decodeURIComponent(notebookAnswerEffectivenessMatch[1]);
    const body = await readJsonBody(request);
    const session = await getSession(context.namespace, id, { create: false });
    if (!session) throw userFacingError("专题会话不存在或已被删除。", 404);
    const messageIndex = Number(body.messageIndex);
    if (!Number.isInteger(messageIndex) || messageIndex < 0 || messageIndex >= session.history.length) {
      throw userFacingError("无效的回答位置。", 400);
    }
    const target = session.history[messageIndex];
    if (!target || target.role !== "assistant") {
      throw userFacingError("只能更新知识库回答的结果确认。", 400);
    }
    const answerEffectiveness = normalizeAnswerEffectiveness(body.answerEffectiveness);
    session.history[messageIndex] = {
      ...target,
      answerEffectiveness,
      evidenceAudit: normalizeEvidenceAudit(body.evidenceAudit) || target.evidenceAudit,
    };
    session.updatedAt = Date.now();
    const notebook = await writeNotebookFromSession(context.namespace, session);
    sendJson(response, 200, { notebook: notebookSummary(notebook), message: session.history[messageIndex] });
    return;
  }

  const notebookMessageFeedbackMatch = url.pathname.match(/^\/api\/notebooks\/([^/]+)\/message-feedback$/);
  if (request.method === "POST" && notebookMessageFeedbackMatch) {
    const id = decodeURIComponent(notebookMessageFeedbackMatch[1]);
    const body = await readJsonBody(request);
    const session = await getSession(context.namespace, id, { create: false });
    if (!session) throw userFacingError("专题会话不存在或已被删除。", 404);
    const messageIndex = Number(body.messageIndex);
    if (!Number.isInteger(messageIndex) || messageIndex < 0 || messageIndex >= session.history.length) {
      throw userFacingError("无效的回答位置。", 400);
    }
    const target = session.history[messageIndex];
    if (!target || target.role !== "assistant") {
      throw userFacingError("只能更新知识库回答的证据反馈。", 400);
    }
    session.history[messageIndex] = {
      ...target,
      evidenceFeedback: normalizeEvidenceFeedback(body.evidenceFeedback),
      evidenceAudit: normalizeEvidenceAudit(body.evidenceAudit) || target.evidenceAudit,
      learningQueue: normalizeLearningQueue(body.learningQueue) || target.learningQueue,
      answerEffectiveness: normalizeAnswerEffectiveness(body.answerEffectiveness) || target.answerEffectiveness,
    };
    session.updatedAt = Date.now();
    const notebook = await writeNotebookFromSession(context.namespace, session);
    sendJson(response, 200, { notebook: notebookSummary(notebook), message: session.history[messageIndex] });
    return;
  }

  if (request.method === "GET" && url.pathname.startsWith("/api/notebooks/")) {
    const id = decodeURIComponent(url.pathname.slice("/api/notebooks/".length));
    sendJson(response, 200, { notebook: await readNotebook(context.namespace, id) });
    return;
  }

  if (request.method === "GET" && url.pathname === "/api/dossiers") {
    sendJson(response, 200, { dossiers: await listDossiers(context.namespace) });
    return;
  }

  if (request.method === "GET" && url.pathname === "/api/dossiers/overview") {
    sendJson(response, 200, { overview: buildDossierOverview(await readAllDossiers(context.namespace)) });
    return;
  }

  if (request.method === "GET" && url.pathname.endsWith("/session") && url.pathname.startsWith("/api/dossiers/")) {
    const id = decodeURIComponent(url.pathname.slice("/api/dossiers/".length, -"/session".length));
    sendJson(response, 200, buildDossierSessionSeed(await readDossier(context.namespace, id)));
    return;
  }

  if (request.method === "POST" && url.pathname.endsWith("/intake") && url.pathname.startsWith("/api/dossiers/")) {
    const id = decodeURIComponent(url.pathname.slice("/api/dossiers/".length, -"/intake".length));
    const body = await readJsonBody(request);
    const dossier = await readDossier(context.namespace, id);
    assertDossierCanAdvance(dossier);
    sendJson(response, 200, { intake: buildProductIntake({ text: body?.text }, dossier) });
    return;
  }

  if (request.method === "POST" && url.pathname.endsWith("/business-verification") && url.pathname.startsWith("/api/dossiers/")) {
    const id = decodeURIComponent(url.pathname.slice("/api/dossiers/".length, -"/business-verification".length));
    const body = await readJsonBody(request);
    if (String(body?.text || "").trim().length < 2) {
      sendJson(response, 400, { error: "请先填写产品材料，再保存业务验证记录。" });
      return;
    }
    assertDossierCanAdvance(await readDossier(context.namespace, id));
    const dossier = await syncDossierToOpenHumanMemory(context, await updateDossierBusinessVerification(context.namespace, id, body));
    const payload = dossierResponsePayload(dossier);
    sendJson(response, dossierResponseStatus(payload), payload);
    return;
  }

  if (request.method === "POST" && url.pathname.endsWith("/experiment-result") && url.pathname.startsWith("/api/dossiers/")) {
    const id = decodeURIComponent(url.pathname.slice("/api/dossiers/".length, -"/experiment-result".length));
    const body = await readJsonBody(request);
    if (String(body?.text || "").trim().length < 2) {
      sendJson(response, 400, { error: "请先填写实验结果，再保存复盘记录。" });
      return;
    }
    assertDossierCanAdvance(await readDossier(context.namespace, id));
    const dossier = await syncDossierToOpenHumanMemory(context, await updateDossierExperimentResult(context.namespace, id, body));
    const payload = dossierResponsePayload(dossier);
    sendJson(response, dossierResponseStatus(payload), payload);
    return;
  }

  if (request.method === "POST" && url.pathname.endsWith("/review") && url.pathname.startsWith("/api/dossiers/")) {
    const id = decodeURIComponent(url.pathname.slice("/api/dossiers/".length, -"/review".length));
    const body = await readJsonBody(request);
    assertDossierCanAdvance(await readDossier(context.namespace, id));
    const dossier = await syncDossierToOpenHumanMemory(context, await updateDossierReview(context.namespace, id, body));
    const payload = dossierResponsePayload(dossier);
    sendJson(response, dossierResponseStatus(payload), payload);
    return;
  }

  if (request.method === "POST" && url.pathname.endsWith("/self-test") && url.pathname.startsWith("/api/dossiers/")) {
    const id = decodeURIComponent(url.pathname.slice("/api/dossiers/".length, -"/self-test".length));
    const body = await readJsonBody(request);
    assertDossierCanAdvance(await readDossier(context.namespace, id));
    const dossier = await syncDossierToOpenHumanMemory(context, await updateDossierSelfTest(context.namespace, id, body));
    const payload = dossierResponsePayload(dossier);
    sendJson(response, dossierResponseStatus(payload), payload);
    return;
  }

  if (request.method === "POST" && url.pathname.endsWith("/evidence-decision") && url.pathname.startsWith("/api/dossiers/")) {
    const id = decodeURIComponent(url.pathname.slice("/api/dossiers/".length, -"/evidence-decision".length));
    const body = await readJsonBody(request);
    if (!Number.isInteger(Number(body?.sourceIndex)) || !["useful", "irrelevant"].includes(body?.decision)) {
      sendJson(response, 400, { error: "请先选择一段来源摘录，再确认它是否有用。" });
      return;
    }
    const previous = await readDossier(context.namespace, id);
    const updated = await updateDossierEvidenceDecision(context.namespace, id, body);
    const dossier = updated.acceptedEvidence.length > 0
      ? await syncDossierToOpenHumanMemory(context, updated)
      : await writeDossier(context.namespace, {
          ...updated,
          openhumanMemory: await skippedMemoryAfterEvidenceRemoval(context, previous),
        });
    const payload = dossierResponsePayload(dossier);
    sendJson(response, dossierResponseStatus(payload), payload);
    return;
  }

  if (request.method === "GET" && url.pathname.startsWith("/api/dossiers/")) {
    const id = decodeURIComponent(url.pathname.slice("/api/dossiers/".length));
    const dossier = await readDossier(context.namespace, id);
    sendJson(response, 200, dossierResponsePayload(dossier));
    return;
  }

  if (request.method === "POST" && url.pathname === "/api/dossiers") {
    const body = await readJsonBody(request);
    const dossier = await syncDossierToOpenHumanMemory(context, await saveDossier(context, body));
    const payload = dossierResponsePayload(dossier);
    sendJson(response, dossierResponseStatus(payload), payload);
    return;
  }

  if (request.method === "DELETE" && url.pathname.startsWith("/api/dossiers/")) {
    const id = decodeURIComponent(url.pathname.slice("/api/dossiers/".length));
    await deleteDossier(context, context.namespace, id);
    sendJson(response, 200, { ok: true });
    return;
  }

  if (request.method === "POST" && url.pathname === "/api/ask") {
    const body = await readJsonBody(request);
    const question = String(body.question || "").trim();
    if (question.length < 2) {
      sendJson(response, 400, { error: "问题太短了，请输入更具体的问题。" });
      return;
    }

    const session = await getSession(context.namespace, body.sessionId);
    const clientHistory = Array.isArray(body.history)
      ? removeCurrentQuestionFromSeedHistory(normalizeSessionHistory(body.history), question)
      : [];
    if (clientHistory.length > 0) {
      session.history = clientHistory;
    }
    const hasIncomingSourceControls = body.sourceControls && typeof body.sourceControls === "object";
    const sessionSourceControls = normalizeSourceControls(session.sourceControls);
    const historySourceControls = sourceControlsFromHistory(session.history);
    const sourceControls = hasIncomingSourceControls
      ? normalizeSourceControls(body.sourceControls)
      : sourceControlsHasAnyValue(sessionSourceControls)
        ? sessionSourceControls
        : historySourceControls;
    session.sourceControls = sourceControls;
    const userSourceControls = normalizeUserSourceControls(body.userSourceControls || session.userSourceControls);
    session.userSourceControls = userSourceControls;
    const retrievalQuery = buildRetrievalQuery(question, session.history, {
      excludedSourceKeys: sourceControls.excludedSourceKeys,
    });
    const scopeLines = [];
    if (sourceControls.allowedAuthors.length > 0) scopeLines.push(`资料范围：${sourceControls.allowedAuthors.join("、")}`);
    if (sourceControls.allowedSourceKeys.length > 0) scopeLines.push(`指定来源：${sourceControls.allowedSourceKeys.slice(0, 8).join("、")}`);
    const scopedRetrievalQuery = scopeLines.length > 0 ? `${retrievalQuery}\n${scopeLines.join("\n")}` : retrievalQuery;
    const authorDiversityAuthors = isAuthorComparisonRequest(`${question}\n${scopedRetrievalQuery}`)
      && sourceControls.allowedSourceKeys.length === 0
      && sourceControls.selectedSources.length === 0
      ? (sourceControls.allowedAuthors.length > 0 ? sourceControls.allowedAuthors : AMAZON_AUTHORS)
      : [];
    const userSourceOnly = userSourceControls.mode === "only" && userSourceControls.enabledIds.length > 0;
    const [retrieval, learningMemoryContext, sourceTreeContext, userSourceContext] = await Promise.all([
      userSourceOnly ? "" : queryKnowledge(context, scopedRetrievalQuery, { authorDiversityAuthors }),
      userSourceOnly ? { data: { context: { chunks: [] } } } : queryLearningMemory(context, scopedRetrievalQuery),
      userSourceOnly
        ? {
            contextText: "",
            calibration: buildSourceTreeCalibration({ query: scopedRetrievalQuery, terms: [], chunkRows: [], summaryRows: [], resolvedSources: [] }),
          }
        : querySourceTreeContext(context, scopedRetrievalQuery, sourceControls),
      queryUserSources(context.namespace, scopedRetrievalQuery, userSourceControls),
    ]);
    const selectedContext = await selectedSourceContextText(context, sourceControls.selectedSources);
    const answerContext = mergeKnowledgeContexts([selectedContext, userSourceContext.contextText, sourceTreeContext.contextText, retrieval]);
    const payloadRetrievalQuestion = userSourceOnly && userSourceContext.contextText
      ? `${scopedRetrievalQuery}\n所选我的资料：\n${userSourceContext.contextText.slice(0, 2200)}`
      : scopedRetrievalQuery;
    const payload = buildQaPayload(question, answerContext, payloadRetrievalQuestion, {
      excludedSourceKeys: sourceControls.excludedSourceKeys,
      allowedAuthors: sourceControls.allowedAuthors,
      allowedSourceKeys: sourceControls.allowedSourceKeys,
      allowedSourceCount: sourceControls.selectedSources.length || undefined,
      productInput: normalizeProductInputForAsk(body.productInput),
      intentPreference: normalizeIntentPreference(body.intentPreference),
      learningMemoryContext,
      sourceTreeCalibration: sourceTreeContext.calibration,
    });
    const userMessage = { role: "user", content: question, createdAt: new Date().toISOString() };
    const assistantMessage = {
      role: "assistant",
      content: payload.answer,
      sources: payload.sources,
      rankedEvidence: payload.rankedEvidence,
      sourceScope: payload.sourceScope,
      productInputSummary: payload.productInputSummary,
      diagnosisPanel: payload.diagnosisPanel,
      validationPack: payload.validationPack,
      evidenceChain: payload.evidenceChain,
      evidenceAudit: payload.evidenceAudit,
      sourceTrust: payload.sourceTrust,
      sourceTreeCalibration: payload.sourceTreeCalibration,
      synthesisAnswer: payload.synthesisAnswer,
      notebookGuide: payload.notebookGuide,
      graph: payload.graph,
      topicSourceTree: payload.topicSourceTree,
      sourceStudyPack: payload.sourceStudyPack,
      authorPerspectiveRoom: payload.authorPerspectiveRoom,
      learningCard: payload.learningCard,
      workflowIntent: payload.workflowIntent,
      learningQueue: payload.learningQueue,
      knowledgeGapRadar: payload.knowledgeGapRadar,
      nextBestSource: payload.nextBestSource,
      usageFootprint: payload.usageFootprint,
      learningMemoryReminder: payload.learningMemoryReminder,
      sourceControls,
      userSourceControls,
      createdAt: new Date().toISOString(),
    };
    session.history.push(userMessage, assistantMessage);
    session.history = session.history.slice(-MAX_SESSION_MESSAGES);
    session.updatedAt = Date.now();
    const notebook = await writeNotebookFromSession(context.namespace, session);

    sendJson(response, 200, {
      ...payload,
      sessionId: session.id,
      notebook: notebookSummary(notebook),
      sourceControls,
      userSourceControls,
      messages: session.history,
    });
    return;
  }

  sendJson(response, 404, { error: "Not found" });
}

async function getSession(namespace, sessionId, options = {}) {
  pruneSessions();
  const requested = typeof sessionId === "string" ? sessionId.trim() : "";
  if (requested && sessions.has(requested)) {
    const existing = sessions.get(requested);
    existing.sourceControls = normalizeSourceControls(existing.sourceControls);
    existing.userSourceControls = normalizeUserSourceControls(existing.userSourceControls);
    return existing;
  }

  if (requested) {
    const restored = await sessionFromNotebook(namespace, requested);
    if (restored) {
      sessions.set(restored.id, restored);
      return restored;
    }
  }

  if (options.create === false) return null;

  const id = safeNotebookIdOrEmpty(requested) || randomUUID();
  const now = Date.now();
  const session = {
    id,
    history: [],
    sourceControls: normalizeSourceControls(),
    userSourceControls: normalizeUserSourceControls(),
    createdAt: new Date(now).toISOString(),
    updatedAt: now,
  };
  sessions.set(id, session);
  return session;
}

function normalizeSessionHistory(history) {
  return history
    .filter((entry) => entry && (entry.role === "user" || entry.role === "assistant"))
    .map((entry) => ({
      role: entry.role,
      content: String(entry.content || "").slice(0, 2500),
      sources: Array.isArray(entry.sources) ? entry.sources.slice(0, 5) : undefined,
      rankedEvidence: normalizeRankedEvidence(entry.rankedEvidence),
      sourceScope: normalizeSourceScope(entry.sourceScope),
      productInputSummary: normalizeProductInputSummary(entry.productInputSummary),
      diagnosisPanel: normalizeDiagnosisPanel(entry.diagnosisPanel),
      validationPack: normalizeValidationPack(entry.validationPack),
      evidenceChain: normalizeEvidenceChain(entry.evidenceChain),
      evidenceAudit: normalizeEvidenceAudit(entry.evidenceAudit),
      answerEffectiveness: normalizeAnswerEffectiveness(entry.answerEffectiveness),
      sourceTrust: normalizeSourceTrust(entry.sourceTrust),
      sourceTreeCalibration: normalizeSourceTreeCalibrationForMessage(entry.sourceTreeCalibration),
      evidenceFeedback: normalizeEvidenceFeedback(entry.evidenceFeedback),
      synthesisAnswer: normalizeSynthesisAnswer(entry.synthesisAnswer),
      notebookGuide: normalizeNotebookGuide(entry.notebookGuide),
      graph: normalizeGraph(entry.graph),
      topicSourceTree: normalizeTopicSourceTree(entry.topicSourceTree),
      sourceStudyPack: normalizeSourceStudyPack(entry.sourceStudyPack),
      authorPerspectiveRoom: normalizeAuthorPerspectiveRoom(entry.authorPerspectiveRoom),
      learningCard: normalizeLearningCard(entry.learningCard),
      workflowIntent: normalizeWorkflowIntent(entry.workflowIntent),
      learningQueue: normalizeLearningQueue(entry.learningQueue),
      knowledgeGapRadar: normalizeKnowledgeGapRadar(entry.knowledgeGapRadar),
      nextBestSource: normalizeNextBestSource(entry.nextBestSource),
      usageFootprint: normalizeUsageFootprint(entry.usageFootprint),
      learningMemoryReminder: normalizeLearningMemoryReminderForMessage(entry.learningMemoryReminder),
      sourceControls: normalizeSourceControls(entry.sourceControls),
      userSourceControls: normalizeUserSourceControls(entry.userSourceControls),
      openhumanMemory: normalizeOpenHumanMemoryForMessage(entry.openhumanMemory),
      savedDossierId: typeof entry.savedDossierId === "string" ? entry.savedDossierId.slice(0, 120) : undefined,
      createdAt: typeof entry.createdAt === "string" ? entry.createdAt : new Date().toISOString(),
    }))
    .slice(-MAX_SESSION_MESSAGES);
}

function safeNotebookId(id) {
  const safe = String(id || "").trim().replace(/[^a-zA-Z0-9_.-]/g, "-").slice(0, 120);
  if (!safe) throw new Error("无效的专题会话编号。");
  return safe;
}

function safeNotebookIdOrEmpty(id) {
  try {
    return safeNotebookId(id);
  } catch {
    return "";
  }
}

async function ensureNotebookDir(namespace) {
  const dir = path.join(NOTEBOOK_ROOT, safeNamespace(namespace));
  await mkdir(dir, { recursive: true });
  return dir;
}

function notebookPath(namespace, id) {
  return path.join(NOTEBOOK_ROOT, safeNamespace(namespace), `${safeNotebookId(id)}.json`);
}

async function ensureUserSourceDir(namespace) {
  const dir = path.join(USER_SOURCE_ROOT, safeNamespace(namespace));
  await mkdir(dir, { recursive: true });
  return dir;
}

function userSourcePath(namespace, id) {
  return path.join(USER_SOURCE_ROOT, safeNamespace(namespace), `${safeUserSourceId(id)}.json`);
}

async function listUserSources(namespace) {
  const dir = await ensureUserSourceDir(namespace);
  const names = await readdir(dir);
  const rows = [];
  for (const name of names.filter((item) => item.endsWith(".json"))) {
    try {
      const raw = await readFile(path.join(dir, name), "utf8");
      rows.push(userSourceSummary(normalizeUserSourceRecord(namespace, JSON.parse(raw))));
    } catch (error) {
      if (process.env.AMAZON_QA_DEBUG) {
        console.warn(`User source skipped: ${name}: ${friendlyError(error)}`);
      }
    }
  }
  return rows.sort((a, b) => Date.parse(b.updatedAt || b.createdAt || 0) - Date.parse(a.updatedAt || a.createdAt || 0)).slice(0, 80);
}

async function saveUserSource(namespace, body = {}) {
  const title = compactServerText(body.title || "我的资料", 120);
  const content = String(body.content || "").trim().slice(0, 180_000);
  if (title.length < 2) throw userFacingError("请给资料写一个标题。", 400);
  if (content.length < 10) throw userFacingError("资料内容太短，请粘贴更完整的内容。", 400);
  const now = new Date().toISOString();
  const id = safeUserSourceId(body.id || userSourceIdFromContent(title, content));
  const existing = await readUserSourceIfExists(namespace, id);
  const record = normalizeUserSourceRecord(namespace, {
    id,
    title,
    content,
    author: USER_SOURCE_AUTHOR,
    createdAt: existing?.createdAt || now,
    updatedAt: now,
  });
  const dir = await ensureUserSourceDir(namespace);
  const file = path.join(dir, `${record.id}.json`);
  const tmp = `${file}.${process.pid}.${Date.now()}.tmp`;
  await writeFile(tmp, `${JSON.stringify(record, null, 2)}\n`, "utf8");
  await rename(tmp, file);
  return userSourceSummary(record);
}

async function readUserSourceIfExists(namespace, id) {
  try {
    const raw = await readFile(userSourcePath(namespace, id), "utf8");
    return normalizeUserSourceRecord(namespace, JSON.parse(raw));
  } catch (error) {
    if (error?.code === "ENOENT") return null;
    throw error;
  }
}

async function deleteUserSource(namespace, id) {
  try {
    await unlink(userSourcePath(namespace, id));
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
}

function normalizeUserSourceRecord(namespace, record = {}) {
  const content = String(record.content || "").trim().slice(0, 180_000);
  const title = compactServerText(record.title || "我的资料", 120);
  const id = safeUserSourceId(record.id || userSourceIdFromContent(title, content));
  const createdAt = typeof record.createdAt === "string" ? record.createdAt : new Date().toISOString();
  const updatedAt = typeof record.updatedAt === "string" ? record.updatedAt : createdAt;
  return {
    id,
    namespace: safeNamespace(record.namespace || namespace),
    title,
    author: USER_SOURCE_AUTHOR,
    content,
    createdAt,
    updatedAt,
  };
}

function userSourceSummary(record) {
  const normalized = normalizeUserSourceRecord(record.namespace || DEFAULT_NAMESPACE, record);
  return {
    id: normalized.id,
    title: normalized.title,
    author: normalized.author,
    excerpt: compactServerText(normalized.content, 180),
    charCount: normalized.content.length,
    createdAt: normalized.createdAt,
    updatedAt: normalized.updatedAt,
  };
}

function safeUserSourceId(id) {
  const safe = String(id || "").trim().replace(/[^a-zA-Z0-9_.-]/g, "-").slice(0, 100);
  if (!safe) throw new Error("无效的用户资料编号。");
  return safe;
}

function userSourceIdFromContent(title, content) {
  return `user-${createHash("sha1").update(`${title}\n${content}`).digest("hex").slice(0, 16)}`;
}

async function ensureLearningNoteDir(namespace) {
  const dir = path.join(LEARNING_NOTE_ROOT, safeNamespace(namespace));
  await mkdir(dir, { recursive: true });
  return dir;
}

function learningNotePath(namespace, id) {
  return path.join(LEARNING_NOTE_ROOT, safeNamespace(namespace), `${safeLearningNoteId(id)}.json`);
}

async function listLearningNotes(namespace) {
  const dir = await ensureLearningNoteDir(namespace);
  const names = await readdir(dir);
  const rows = [];
  for (const name of names.filter((item) => item.endsWith(".json"))) {
    try {
      const raw = await readFile(path.join(dir, name), "utf8");
      rows.push(learningNoteSummary(normalizeLearningNoteRecord(namespace, JSON.parse(raw))));
    } catch (error) {
      if (process.env.AMAZON_QA_DEBUG) {
        console.warn(`Learning note skipped: ${name}: ${friendlyError(error)}`);
      }
    }
  }
  return rows.sort((a, b) => Date.parse(b.updatedAt || b.createdAt || 0) - Date.parse(a.updatedAt || a.createdAt || 0)).slice(0, 120);
}

async function saveLearningNoteRecord(namespace, body = {}) {
  const content = String(body.content || "").trim().slice(0, 160_000);
  const title = compactServerText(body.title || titleFromLearningNoteContent(content), 140);
  if (title.length < 2) throw userFacingError("请给笔记写一个标题。", 400);
  if (content.length < 3) throw userFacingError("笔记内容太短，请先写一点内容。", 400);
  const now = new Date().toISOString();
  const id = safeLearningNoteId(body.id || learningNoteIdFromContent(title, content));
  const existing = await readLearningNoteIfExists(namespace, id);
  const record = normalizeLearningNoteRecord(namespace, {
    id,
    title,
    content,
    origin: body.origin,
    source: body.source,
    boundary: LEARNING_NOTE_BOUNDARY,
    createdAt: existing?.createdAt || now,
    updatedAt: now,
  });
  const dir = await ensureLearningNoteDir(namespace);
  const file = path.join(dir, `${record.id}.json`);
  const tmp = `${file}.${process.pid}.${Date.now()}.tmp`;
  await writeFile(tmp, `${JSON.stringify(record, null, 2)}\n`, "utf8");
  await rename(tmp, file);
  return learningNoteSummary(record);
}

async function readLearningNoteIfExists(namespace, id) {
  try {
    const raw = await readFile(learningNotePath(namespace, id), "utf8");
    return normalizeLearningNoteRecord(namespace, JSON.parse(raw));
  } catch (error) {
    if (error?.code === "ENOENT") return null;
    throw error;
  }
}

async function deleteLearningNote(namespace, id) {
  try {
    await unlink(learningNotePath(namespace, id));
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
}

async function convertLearningNoteToUserSource(namespace, id) {
  const note = await readLearningNoteIfExists(namespace, id);
  if (!note) throw userFacingError("这条学习笔记不存在或已被删除。", 404);
  return saveUserSource(namespace, {
    id: `note-${note.id}`.slice(0, 100),
    title: `笔记：${note.title}`,
    content: learningNoteAsUserSourceContent(note),
  });
}

function normalizeLearningNoteRecord(namespace, record = {}) {
  const content = String(record.content || "").trim().slice(0, 160_000);
  const title = compactServerText(record.title || titleFromLearningNoteContent(content), 140);
  const id = safeLearningNoteId(record.id || learningNoteIdFromContent(title, content));
  const source = normalizeLearningNoteSource(record.source);
  const createdAt = typeof record.createdAt === "string" ? record.createdAt : new Date().toISOString();
  const updatedAt = typeof record.updatedAt === "string" ? record.updatedAt : createdAt;
  return {
    id,
    namespace: safeNamespace(record.namespace || namespace),
    title,
    content,
    origin: record.origin === "answer" ? "answer" : "manual",
    source,
    boundary: compactServerText(record.boundary || LEARNING_NOTE_BOUNDARY, 360),
    createdAt,
    updatedAt,
  };
}

function normalizeLearningNoteSource(source) {
  if (!source || typeof source !== "object") return {};
  const messageIndex = Number(source.messageIndex);
  return {
    sessionId: compactServerText(source.sessionId, 120),
    messageIndex: Number.isInteger(messageIndex) ? messageIndex : undefined,
    question: compactServerText(source.question, 500),
  };
}

function learningNoteSummary(record) {
  const normalized = normalizeLearningNoteRecord(record.namespace || DEFAULT_NAMESPACE, record);
  return {
    id: normalized.id,
    title: normalized.title,
    excerpt: compactServerText(normalized.content, 220),
    charCount: normalized.content.length,
    origin: normalized.origin,
    source: normalized.source,
    boundary: normalized.boundary,
    createdAt: normalized.createdAt,
    updatedAt: normalized.updatedAt,
  };
}

function safeLearningNoteId(id) {
  const safe = String(id || "").trim().replace(/[^a-zA-Z0-9_.-]/g, "-").slice(0, 100);
  if (!safe) throw new Error("无效的学习笔记编号。");
  return safe;
}

function learningNoteIdFromContent(title, content) {
  return `note-${createHash("sha1").update(`${title}\n${content}`).digest("hex").slice(0, 16)}`;
}

function titleFromLearningNoteContent(content) {
  const firstLine = String(content || "").split(/\n+/).map((line) => line.trim()).find(Boolean) || "学习笔记";
  return compactServerText(firstLine.replace(/^#+\s*/, ""), 80);
}

function learningNoteAsUserSourceContent(note) {
  const sourceLines = [];
  if (note.source?.question) sourceLines.push(`原问题：${note.source.question}`);
  if (note.origin === "answer") sourceLines.push("来源：由一次问答回答保存为学习笔记");
  else sourceLines.push("来源：用户手写学习笔记");
  return [
    "资料类型：我的学习笔记",
    "边界：这不是三位作者的原文证据；它只代表用户自己的整理。回答时请明确按“我的资料”引用。",
    ...sourceLines,
    "",
    note.content,
  ].join("\n");
}

async function listNotebooks(namespace) {
  const dir = await ensureNotebookDir(namespace);
  let names = [];
  try {
    names = await readdir(dir);
  } catch {
    return { notebooks: [], hiddenTestNotebookCount: 0 };
  }
  const notebooks = [];
  let hiddenTestNotebookCount = 0;
  for (const name of names.filter((item) => item.endsWith(".json")).slice(0, 260)) {
    const notebookId = name.replace(/\.json$/i, "");
    if (isTestNotebookId(notebookId)) {
      hiddenTestNotebookCount += 1;
      continue;
    }
    try {
      const raw = await readFile(path.join(dir, name), "utf8");
      notebooks.push(notebookSummary(normalizeNotebookRecord(namespace, JSON.parse(raw))));
    } catch (error) {
      if (process.env.AMAZON_QA_DEBUG) {
        console.warn(`Notebook skipped: ${name}: ${friendlyError(error)}`);
      }
    }
  }
  return {
    notebooks: notebooks
      .sort((a, b) => Date.parse(b.updatedAt || b.createdAt || 0) - Date.parse(a.updatedAt || a.createdAt || 0))
      .slice(0, 50),
    hiddenTestNotebookCount,
  };
}

function isTestNotebookId(id) {
  const value = String(id || "").trim();
  return TEST_NOTEBOOK_ID_PATTERNS.some((pattern) => pattern.test(value));
}

async function readNotebook(namespace, id) {
  const safeId = safeNotebookId(id);
  try {
    const raw = await readFile(notebookPath(namespace, safeId), "utf8");
    return normalizeNotebookRecord(namespace, JSON.parse(raw));
  } catch (error) {
    if (error?.code === "ENOENT") {
      throw userFacingError("专题会话不存在或已被删除。", 404);
    }
    throw error;
  }
}

async function sessionFromNotebook(namespace, id) {
  try {
    const notebook = await readNotebook(namespace, id);
    return {
      id: notebook.id,
      history: normalizeSessionHistory(notebook.messages),
      sourceControls: normalizeSourceControls(notebook.sourceControls),
      userSourceControls: normalizeUserSourceControls(notebook.userSourceControls),
      createdAt: notebook.createdAt,
      updatedAt: Date.parse(notebook.updatedAt || notebook.createdAt || "") || Date.now(),
    };
  } catch (error) {
    if (error?.statusCode === 404) return null;
    throw error;
  }
}

async function writeNotebookFromSession(namespace, session) {
  const record = normalizeNotebookRecord(namespace, {
    id: session.id,
    title: notebookTitleFromMessages(session.history),
    topic: notebookTopicFromMessages(session.history),
    boundary: NOTEBOOK_BOUNDARY,
    sourceControls: normalizeSourceControls(session.sourceControls),
    userSourceControls: normalizeUserSourceControls(session.userSourceControls),
    messages: normalizeSessionHistory(session.history),
    createdAt: session.createdAt || new Date().toISOString(),
    updatedAt: new Date(session.updatedAt || Date.now()).toISOString(),
  });
  const dir = await ensureNotebookDir(namespace);
  const file = path.join(dir, `${record.id}.json`);
  const tmp = `${file}.${process.pid}.${Date.now()}.tmp`;
  await writeFile(tmp, `${JSON.stringify(record, null, 2)}\n`, "utf8");
  await rename(tmp, file);
  return record;
}

function normalizeNotebookRecord(namespace, record = {}) {
  const messages = normalizeSessionHistory(Array.isArray(record.messages) ? record.messages : record.history || []);
  const id = safeNotebookId(record.id || randomUUID());
  const createdAt = typeof record.createdAt === "string" ? record.createdAt : new Date().toISOString();
  const updatedAt = typeof record.updatedAt === "string" ? record.updatedAt : createdAt;
  return {
    id,
    namespace: safeNamespace(record.namespace || namespace),
    title: compactServerText(record.title || notebookTitleFromMessages(messages), 120),
    topic: compactServerText(record.topic || notebookTopicFromMessages(messages), 180),
    boundary: compactServerText(record.boundary || NOTEBOOK_BOUNDARY, 260),
    sourceControls: normalizeSourceControls(record.sourceControls),
    userSourceControls: normalizeUserSourceControls(record.userSourceControls),
    messages,
    createdAt,
    updatedAt,
  };
}

function notebookSummary(notebook) {
  const record = normalizeNotebookRecord(notebook.namespace || DEFAULT_NAMESPACE, notebook);
  const assistantMessages = record.messages
    .filter((message) => message.role === "assistant")
    .filter((message) => !notebookMessageNeedsRecheck(message));
  const sourceKeys = new Set(collectNotebookSources(assistantMessages, notebookStudyPackExcludedSourceKeys(record)).map((source) => source.key));
  return {
    id: record.id,
    title: record.title,
    topic: record.topic,
    boundary: record.boundary,
    createdAt: record.createdAt,
    updatedAt: record.updatedAt,
    messageCount: record.messages.length,
    sourceCount: sourceKeys.size,
  };
}

function buildNotebookStudyPack(notebook) {
  const record = normalizeNotebookRecord(notebook.namespace || DEFAULT_NAMESPACE, notebook);
  const excludedSourceKeys = notebookStudyPackExcludedSourceKeys(record);
  const assistantMessages = record.messages
    .filter((message) => message.role === "assistant")
    .filter((message) => !notebookMessageNeedsRecheck(message));
  const questions = record.messages
    .filter((message) => message.role === "user" && message.content)
    .map((message) => compactServerText(message.content, 160))
    .slice(0, 8);
  const sourceLedger = collectNotebookSources(assistantMessages, excludedSourceKeys);
  const takeaways = collectNotebookTakeaways(assistantMessages, excludedSourceKeys);
  const checklist = collectNotebookChecklist(assistantMessages);
  const reviewQuestions = collectNotebookReviewQuestions(assistantMessages);
  const concepts = collectNotebookConcepts(assistantMessages);
  const graphStats = assistantMessages.reduce((stats, message) => ({
    nodes: stats.nodes + (Array.isArray(message.graph?.nodes) ? message.graph.nodes.length : 0),
    edges: stats.edges + (Array.isArray(message.graph?.edges) ? message.graph.edges.length : 0),
  }), { nodes: 0, edges: 0 });
  const pack = {
    id: record.id,
    title: record.title,
    topic: record.topic,
    boundary: `${NOTEBOOK_BOUNDARY} 当前学习包是本地文字预览：问答和引用可用，完整来源树学习仍可能处理中；不包含音频或视频功能。`,
    updatedAt: record.updatedAt,
    overview: {
      questions,
      answerCount: assistantMessages.length,
      sourceCount: sourceLedger.length,
      conceptCount: concepts.length,
      graphNodes: graphStats.nodes,
      graphEdges: graphStats.edges,
      status: sourceLedger.length > 0 ? "source_backed" : "needs_source",
      statusLabel: sourceLedger.length > 0 ? "已绑定可采纳作者原文" : "需要补可采纳作者原文",
    },
    takeaways,
    checklist,
    reviewQuestions,
    concepts,
    sourceLedger,
  };
  const studio = buildNotebookStudioPack(pack);
  return {
    ...pack,
    studio,
    markdown: notebookStudyPackMarkdown(pack),
    exportMarkdown: notebookStudioExportMarkdown(pack, studio),
    exportJson: `${JSON.stringify({
      id: pack.id,
      title: pack.title,
      topic: pack.topic,
      boundary: pack.boundary,
      overview: pack.overview,
      takeaways: pack.takeaways,
      checklist: pack.checklist,
      reviewQuestions: pack.reviewQuestions,
      concepts: pack.concepts,
      sourceLedger: pack.sourceLedger,
      studio,
    }, null, 2)}\n`,
  };
}

function buildNotebookStudioPack(pack) {
  const sourceTable = (pack.sourceLedger || []).map((source, index) => ({
    index: index + 1,
    author: source.author || "",
    date: source.date || "",
    title: source.title || "",
    claim: source.claimLabel || "",
    excerpt: source.excerpt || "",
    sourceUrl: source.sourceUrl || "",
    sourcePath: source.sourcePath || "",
    identity: source.identity || "作者原文证据来源",
  }));
  const flashcards = (pack.reviewQuestions || []).map((item, index) => ({
    id: `card-${index + 1}`,
    front: item.question || "",
    back: item.answer || item.prompt || "回到专题来源和学习要点复核。",
    identity: item.identity || "复习题",
    boundary: item.boundary || "复习题用于检查理解，不会写入原始知识库。",
  }));
  const mindMap = buildNotebookMindMap(pack);
  const masteryQuiz = buildNotebookMasteryQuiz(pack);
  const actionPlan = buildNotebookActionPlan(pack);
  const reportSections = [
    {
      title: "核心结论",
      items: (pack.takeaways || []).slice(0, 6).map((item) => `${item.label}：${item.text}`),
    },
    {
      title: "行动清单",
      items: (pack.checklist || []).slice(0, 6).map((item) => `${item.label}${item.reason ? `：${item.reason}` : ""}`),
    },
    {
      title: "复习问题",
      items: (pack.reviewQuestions || []).slice(0, 6).map((item) => item.question),
    },
    {
      title: "来源账本",
      items: sourceTable.slice(0, 8).map((source) => `${source.author}《${source.title}》`),
    },
  ].filter((section) => section.items.length > 0);
  return {
    title: `${pack.title || "亚马逊学习专题"} · 本地学习包预览`,
    boundary: "本地学习包预览基于本专题来源和问答做文字整理；作者原文证据仍以来源账本为准，完整来源树学习可能仍在后台处理，不包含音频或视频功能。",
    reportSections,
    mindMap,
    masteryQuiz,
    actionPlan,
    flashcards,
    sourceTable,
    sourceTableCsv: csvRows(["序号", "作者", "日期", "标题", "证据类型", "摘录", "原文链接", "来源文件"], sourceTable.map((source) => [
      source.index,
      source.author,
      source.date,
      source.title,
      source.identity,
      source.excerpt,
      source.sourceUrl,
      source.sourcePath,
    ])),
    flashcardsCsv: csvRows(["正面", "背面", "身份", "边界"], flashcards.map((card) => [
      card.front,
      card.back,
      card.identity,
      card.boundary,
    ])),
  };
}

function buildNotebookMasteryQuiz(pack) {
  const reviewQuestions = Array.isArray(pack.reviewQuestions) ? pack.reviewQuestions : [];
  const takeaways = Array.isArray(pack.takeaways) ? pack.takeaways : [];
  const checklist = Array.isArray(pack.checklist) ? pack.checklist : [];
  const concepts = Array.isArray(pack.concepts) ? pack.concepts : [];
  const sources = Array.isArray(pack.sourceLedger) ? pack.sourceLedger : [];
  const sourceLedgerIndexByKey = new Map(sources.map((source, index) => [source.key, index]).filter(([key]) => key));
  const items = [];
  const seen = new Set();
  const validSourceIndexes = (indexes = []) => [...new Set(indexes)]
    .filter((index) => Number.isInteger(index) && index >= 0 && index < sources.length)
    .slice(0, 4);
  const sourceIndexesFromKeys = (keys = []) => validSourceIndexes(
    [...new Set(keys)].map((key) => sourceLedgerIndexByKey.get(key)).filter(Number.isInteger),
  );
  const add = ({ question, expectedAnswer = "", explanation = "", sourceIndexes = [], kind = "open" }) => {
    const safeQuestion = compactServerText(question, 180);
    if (!safeQuestion || seen.has(safeQuestion)) return;
    seen.add(safeQuestion);
    const safeExpectedAnswer = compactServerText(expectedAnswer, 360);
    const stableId = createHash("sha1").update(`${kind}\n${safeQuestion}\n${safeExpectedAnswer}`).digest("hex").slice(0, 12);
    items.push({
      id: `quiz-${stableId}`,
      kind,
      question: safeQuestion,
      expectedAnswer: safeExpectedAnswer,
      explanation: compactServerText(explanation, 260),
      sourceIndexes: validSourceIndexes(sourceIndexes),
      identity: "理解自测",
      canUseAsEvidence: false,
      boundary: "自测只记录你的理解状态，不会写入作者原文证据，也不会自动保存为学习结论。",
    });
  };

  reviewQuestions.slice(0, 4).forEach((item) => add({
    question: item.question,
    expectedAnswer: item.answer || item.prompt || "回到本地学习包预览和来源账本复核。",
    explanation: item.boundary || "复习题用于检查理解，不会写入原始知识库。",
    kind: "review",
  }));
  takeaways.slice(0, 4).forEach((item) => add({
    question: `用自己的话复述：${item.label || "这个要点"}为什么会影响你的亚马逊判断？`,
    expectedAnswer: item.text || "先回到学习要点复核。",
    explanation: item.support === "source_backed"
      ? "这道题检查你能否把来源支撑的要点转成自己的判断。"
      : "这道题检查你是否能识别待补来源的系统整理。",
    sourceIndexes: item.support === "source_backed" ? sourceIndexesFromKeys(item.sourceKeys) : [],
    kind: "explain",
  }));
  checklist.slice(0, 3).forEach((item) => add({
    question: `如果现在要执行「${item.label || "这一步"}」，你会先看哪项业务数据？`,
    expectedAnswer: item.reason || "先补产品、关键词、点击率、转化率或广告数据，再决定是否执行。",
    explanation: item.boundary || "行动建议需要用你的产品数据验证。",
    kind: "action",
  }));
  concepts.slice(0, 3).forEach((item) => add({
    question: `把「${item.label || "这个概念"}」放到你的产品里，它对应哪个可观察信号？`,
    expectedAnswer: "用一个可观察信号回答，例如主图点击率、转化率、关键词排名、广告点击或页面停留。",
    explanation: item.boundary || "概念节点是学习索引，不是作者原文。",
    kind: "concept",
  }));
  if (!items.length) {
    add({
      question: sources.length ? "这个专题最先应该核对哪一条来源？" : "暂无来源支撑时，这个专题下一步应该怎么做？",
      expectedAnswer: sources.length
        ? `先打开来源账本第 1 条：${[sources[0].author, sources[0].title].filter(Boolean).join(" · ") || "作者原文来源"}，确认上下文是否支撑当前判断。`
        : "先继续追问或补充资料，拿到可采纳作者原文后再形成学习结论。",
      explanation: "掌握度自测优先检查来源意识和下一步动作。",
      sourceIndexes: sources.length ? [0] : [],
      kind: "source_check",
    });
  }

  return {
    title: "掌握度自测",
    boundary: "自测只检查你对本专题的理解，不会写入作者原文证据，也不会自动保存为学习结论。",
    scoring: "点“掌握”或“再练”只保存在当前浏览器，用来决定下一步复习。",
    items: items.slice(0, 8),
  };
}

function buildNotebookActionPlan(pack) {
  const takeaways = Array.isArray(pack.takeaways) ? pack.takeaways : [];
  const checklist = Array.isArray(pack.checklist) ? pack.checklist : [];
  const sources = Array.isArray(pack.sourceLedger) ? pack.sourceLedger : [];
  const concepts = Array.isArray(pack.concepts) ? pack.concepts : [];
  const sourceLedgerIndexByKey = new Map(sources.map((source, index) => [source.key, index]).filter(([key]) => key));
  const sourceIndexesFromKeys = (keys = []) => [...new Set(keys)]
    .map((key) => sourceLedgerIndexByKey.get(key))
    .filter((index) => Number.isInteger(index) && index >= 0 && index < sources.length)
    .slice(0, 4);
  const conceptText = concepts.map((item) => item.label).filter(Boolean).join("、");
  const dataNeedsFor = (text = "") => {
    const value = `${text} ${conceptText}`;
    const needs = new Set(["当前产品/ASIN", "核心关键词", "近 7-14 天 CTR 与 CVR"]);
    if (/主图|图片|视觉|点击/.test(value)) needs.add("主图、搜索结果位和竞品主图截图");
    if (/广告|ACOS|CPC|SBV|SP/.test(value)) needs.add("广告位置、花费、点击、转化和 ACOS");
    if (/Listing|文案|标题|页面|转化/.test(value)) needs.add("标题、五点、A+、价格、评价和页面转化数据");
    if (/关键词|搜索词|收录|排名/.test(value)) needs.add("搜索词报告、自然排名和收录状态");
    return [...needs].slice(0, 6);
  };
  const steps = [];
  const addStep = ({ label, purpose = "", requiredData = [], sourceIndexes = [], successSignal = "" }) => {
    const safeLabel = compactServerText(label, 150);
    if (!safeLabel || steps.some((item) => item.label === safeLabel)) return;
    steps.push({
      id: `action-${steps.length + 1}`,
      label: safeLabel,
      purpose: compactServerText(purpose || "把学习要点转成可检查的业务动作。", 240),
      requiredData: requiredData.map((item) => compactServerText(item, 80)).filter(Boolean).slice(0, 6),
      successSignal: compactServerText(successSignal || "用 CTR、CVR、广告效率或页面转化变化判断，不凭感觉采纳。", 220),
      sourceIndexes: [...new Set(sourceIndexes)].filter((index) => Number.isInteger(index) && index >= 0 && index < sources.length).slice(0, 4),
      identity: "行动实验计划",
      canUseAsEvidence: false,
      boundary: "行动计划是系统整理和业务验证建议，不是作者原文证据；执行前要回到来源账本和你的产品数据。",
    });
  };

  checklist.slice(0, 5).forEach((item) => addStep({
    label: item.label,
    purpose: item.reason || item.boundary,
    requiredData: dataNeedsFor(`${item.label || ""} ${item.reason || ""}`),
    successSignal: item.kind === "experiment" ? item.reason : "",
  }));
  takeaways.slice(0, 5).forEach((item) => addStep({
    label: `验证：${item.label || "学习要点"}`,
    purpose: item.text,
    requiredData: dataNeedsFor(`${item.label || ""} ${item.text || ""}`),
    successSignal: item.support === "source_backed"
      ? "如果业务数据变化与来源支撑方向一致，再考虑采纳为你的方法。"
      : "先补来源或重新追问，不能直接执行。",
    sourceIndexes: item.support === "source_backed" ? sourceIndexesFromKeys(item.sourceKeys) : [],
  }));
  if (!steps.length) {
    addStep({
      label: sources.length ? "先核对来源账本" : "先补可采纳来源",
      purpose: sources.length ? "从来源账本确认哪些原文真的支撑当前问题。" : "当前专题还没有足够作者原文，先继续追问或补资料。",
      requiredData: sources.length ? ["来源账本第 1 条原文上下文", "当前问题和你的产品数据"] : ["更具体的问题", "新的作者原文或你的业务材料"],
      successSignal: sources.length ? "至少确认 1 条可采纳原文，再进入业务验证。" : "找到可采纳作者原文后再生成行动计划。",
      sourceIndexes: sources.length ? [0] : [],
    });
  }

  return {
    title: "亚马逊行动实验计划",
    boundary: "行动实验计划只把学习包转成业务验证路径，不会写入作者原文证据，也不会自动保存为学习档案。",
    summary: "先核对来源，再补业务数据，最后用小实验验证是否适用于你的产品。",
    steps: steps.slice(0, 8),
  };
}

function buildNotebookMindMap(pack) {
  const nodes = [{ id: "topic", type: "topic", label: pack.title || "亚马逊学习专题" }];
  const edges = [];
  const addNode = (id, type, label) => {
    const safeLabel = compactServerText(label, 90);
    if (!safeLabel || nodes.some((node) => node.id === id)) return;
    nodes.push({ id, type, label: safeLabel });
  };
  (pack.concepts || []).slice(0, 10).forEach((concept, index) => {
    const id = `concept-${index + 1}`;
    addNode(id, "concept", concept.label);
    edges.push({ from: "topic", to: id, type: "contains" });
  });
  (pack.takeaways || []).slice(0, 6).forEach((item, index) => {
    const id = `takeaway-${index + 1}`;
    addNode(id, "takeaway", item.label || item.text);
    edges.push({ from: "topic", to: id, type: item.support === "source_backed" ? "source_backed" : "needs_source" });
  });
  (pack.sourceLedger || []).slice(0, 8).forEach((source, index) => {
    const id = `source-${index + 1}`;
    addNode(id, "source", `${source.author || "未知作者"}《${source.title || "未命名来源"}》`);
    edges.push({ from: "topic", to: id, type: "supported_by" });
  });
  return { nodes, edges };
}

function collectNotebookSources(messages, excludedSourceKeys = new Set()) {
  const seen = new Set();
  const rows = [];
  messages.forEach((message, messageIndex) => {
    for (const claim of acceptedNotebookSourceClaims(message, excludedSourceKeys)) {
      const sourceIndex = claim.sourceIndex;
      const source = message.sources?.[sourceIndex];
      if (!source) continue;
      const key = notebookSourceKey(source);
      if (!key.trim() || seen.has(key)) continue;
      seen.add(key);
      rows.push({
        id: `m${messageIndex}:source${sourceIndex}`,
        key,
        identity: "作者原文证据来源",
        claimId: compactServerText(claim.id, 80),
        claimLabel: compactServerText(claim.label || "证据", 80),
        author: compactServerText(source.author, 80),
        date: compactServerText(source.date, 32),
        title: compactServerText(source.title || "未命名来源", 160),
        sourceUrl: compactServerText(source.sourceUrl, 260),
        sourcePath: compactServerText(source.sourcePath, 260),
        excerpt: compactServerText(claim.quote || claim.text || source.excerpt, 360),
      });
    }
  });
  return rows.slice(0, 24);
}

function collectNotebookTakeaways(messages, excludedSourceKeys = new Set()) {
  const rows = [];
  const add = (message, label, text, sourceIndexes = []) => {
    const safeText = compactServerText(text, 260);
    if (!safeText || rows.some((item) => item.text === safeText)) return;
    const validSourceIndexes = validNotebookSourceIndexes(message, sourceIndexes, excludedSourceKeys);
    const sourceKeys = validSourceIndexes
      .map((index) => notebookSourceKey(message.sources?.[index]))
      .filter(Boolean)
      .slice(0, 5);
    const hasSourceSupport = validSourceIndexes.length > 0;
    rows.push({
      label: compactServerText(label || "学习要点", 80),
      text: safeText,
      identity: hasSourceSupport ? "来源支撑的系统整理" : "未绑定来源的系统整理",
      boundary: hasSourceSupport
        ? "已绑定本专题可采纳作者原文；采纳前仍建议打开来源核对。"
        : "这条是系统整理，未绑定可采纳作者原文，只能作为待验证思路。",
      sourceIndexes: validSourceIndexes,
      sourceKeys,
      support: hasSourceSupport ? "source_backed" : "needs_source",
    });
  };

  for (const message of messages) {
    (message.synthesisAnswer?.points || []).forEach((item) => add(message, item.label, item.text, item.sourceIndexes));
    (message.notebookGuide?.briefing || []).forEach((item) => add(message, item.label, item.text, item.sourceIndexes));
    (message.learningCard?.conclusions || []).forEach((item) => add(message, "学习卡结论", item));
    (message.evidenceChain?.claims || [])
      .filter((claim) => claim.type === "source_evidence")
      .filter((claim) => isAcceptedNotebookSourceClaim(message, claim, excludedSourceKeys))
      .forEach((claim) => add(message, claim.label || "原文证据支持的要点", claim.text || claim.quote, [claim.sourceIndex]));
  }
  return rows.slice(0, 10);
}

function notebookStudyPackExcludedSourceKeys(record) {
  const keys = new Set(normalizeSourceControls(record.sourceControls).excludedSourceKeys);
  for (const message of record.messages || []) {
    for (const key of normalizeSourceControls(message?.sourceControls).excludedSourceKeys) {
      keys.add(key);
    }
  }
  return keys;
}

function notebookMessageNeedsRecheck(message) {
  const feedback = message?.evidenceAudit?.feedback || "";
  return feedback === "citation_wrong" || feedback === "retry";
}

function acceptedNotebookSourceClaims(message, excludedSourceKeys = new Set()) {
  return (message?.evidenceChain?.claims || [])
    .filter((claim) => claim?.type === "source_evidence")
    .filter((claim) => isAcceptedNotebookSourceClaim(message, claim, excludedSourceKeys));
}

function isAcceptedNotebookSourceClaim(message, claim, excludedSourceKeys = new Set()) {
  if (!Number.isInteger(claim?.sourceIndex)) return false;
  const feedback = claim?.id ? message?.evidenceFeedback?.[claim.id] : "";
  if (feedback === "irrelevant") return false;
  const source = message?.sources?.[claim.sourceIndex];
  if (!source) return false;
  return !notebookSourceExcluded(source, excludedSourceKeys);
}

function validNotebookSourceIndexes(message, sourceIndexes = [], excludedSourceKeys = new Set()) {
  if (!Array.isArray(sourceIndexes)) return [];
  const accepted = new Set(acceptedNotebookSourceClaims(message, excludedSourceKeys).map((claim) => claim.sourceIndex));
  return [...new Set(sourceIndexes.filter((index) => Number.isInteger(index) && accepted.has(index)))].slice(0, 5);
}

function notebookSourceExcluded(source, excludedSourceKeys = new Set()) {
  if (!(excludedSourceKeys instanceof Set) || excludedSourceKeys.size === 0) return false;
  return sourceIdentityKeysForControl(source).some((key) => excludedSourceKeys.has(key));
}

function notebookSourceKey(source) {
  return sourceIdentityKeysForControl(source)[0] || "";
}

function collectNotebookChecklist(messages) {
  const rows = [];
  const add = (label, reason = "", kind = "action") => {
    const safeLabel = compactServerText(label, 160);
    if (!safeLabel || rows.some((item) => item.label === safeLabel)) return;
    rows.push({
      label: safeLabel,
      reason: compactServerText(reason, 220),
      kind: compactServerText(kind, 40),
      identity: "行动建议",
      boundary: "行动建议需要用你的产品数据验证，不等同于作者原文结论。",
    });
  };

  for (const message of messages) {
    (message.learningQueue?.items || []).forEach((item) => add(item.label, item.reason, item.kind));
    (message.learningCard?.nextActions || []).forEach((item) => add(item, "", "next_action"));
    (message.validationPack?.dataRequests || []).forEach((item) => add(item.label, item.why, "data"));
    (message.validationPack?.experiments || []).forEach((item) => add(item.title, item.successSignal, "experiment"));
  }
  return rows.slice(0, 12);
}

function collectNotebookReviewQuestions(messages) {
  const rows = [];
  const add = (question, answer = "", prompt = "") => {
    const safeQuestion = compactServerText(question, 180);
    if (!safeQuestion || rows.some((item) => item.question === safeQuestion)) return;
    rows.push({
      question: safeQuestion,
      answer: compactServerText(answer, 260),
      prompt: compactServerText(prompt, 420),
      identity: "复习题",
      boundary: "复习题用于检查理解，不会写入原始知识库。",
    });
  };

  for (const message of messages) {
    (message.learningCard?.studyChecks || []).forEach((item) => add(item.question, item.expectedAnswer, item.prompt));
    (message.notebookGuide?.faq || []).forEach((item) => add(item.question, item.answer, item.prompt));
    (message.notebookGuide?.quiz || []).forEach((item) => add(item.question, item.answer, item.prompt));
  }
  return rows.slice(0, 10);
}

function collectNotebookConcepts(messages) {
  const seen = new Set();
  const concepts = [];
  for (const message of messages) {
    const nodes = Array.isArray(message.graph?.nodes) ? message.graph.nodes : [];
    for (const node of nodes) {
      if (node?.type !== "concept" || !node.label) continue;
      const label = compactServerText(node.label, 60);
      if (!label || seen.has(label)) continue;
      seen.add(label);
      concepts.push({ label, identity: "系统整理", boundary: "概念节点来自本轮问答图谱，不是作者原文。" });
      if (concepts.length >= 16) return concepts;
    }
  }
  return concepts;
}

function notebookStudyPackMarkdown(pack) {
  const lines = [];
  lines.push(`# ${pack.title || "亚马逊学习专题"}`);
  lines.push("");
  lines.push(`专题：${pack.topic || ""}`);
  lines.push(`更新时间：${pack.updatedAt || ""}`);
  lines.push("");
  lines.push(`边界：${pack.boundary || NOTEBOOK_BOUNDARY}`);
  lines.push("");
  lines.push("## 本轮问题");
  for (const question of pack.overview.questions || []) lines.push(`- ${question}`);
  lines.push("");
  lines.push("## 学习要点");
  for (const item of pack.takeaways || []) lines.push(`- ${item.label}：${item.text}（${item.identity}，采纳前回到来源核对）`);
  lines.push("");
  lines.push("## 执行清单");
  for (const item of pack.checklist || []) lines.push(`- ${item.label}${item.reason ? `：${item.reason}` : ""}`);
  lines.push("");
  lines.push("## 复习题");
  for (const item of pack.reviewQuestions || []) lines.push(`- 问：${item.question}${item.answer ? `\n  答：${item.answer}` : ""}`);
  lines.push("");
  lines.push("## 来源账本");
  for (const source of pack.sourceLedger || []) {
    lines.push(`- ${[source.author, source.date, source.title].filter(Boolean).join(" · ")}${source.sourceUrl ? `\n  ${source.sourceUrl}` : ""}`);
  }
  lines.push("");
  lines.push("## 概念");
  if (pack.concepts?.length) lines.push(pack.concepts.map((item) => item.label).join("、"));
  return `${lines.join("\n").trim()}\n`;
}

function notebookStudioExportMarkdown(pack, studio) {
  const lines = [];
  lines.push(notebookStudyPackMarkdown(pack).trim());
  lines.push("");
  lines.push("## 本地学习包预览");
  lines.push("");
  lines.push(`边界：${studio.boundary}`);
  for (const section of studio.reportSections || []) {
    lines.push("");
    lines.push(`### ${section.title}`);
    for (const item of section.items || []) lines.push(`- ${item}`);
  }
  if (studio.flashcards?.length) {
    lines.push("");
    lines.push("### 复习卡");
    for (const card of studio.flashcards) {
      lines.push(`- 正面：${card.front}`);
      lines.push(`  背面：${card.back}`);
    }
  }
  if (studio.masteryQuiz?.items?.length) {
    lines.push("");
    lines.push("### 系统掌握度自测（非作者证据）");
    lines.push(`边界：${studio.masteryQuiz.boundary}`);
    lines.push(`计分：${studio.masteryQuiz.scoring}`);
    for (const item of studio.masteryQuiz.items) {
      lines.push(`- 问：${item.question}`);
      lines.push(`  参考：${item.expectedAnswer || "回到专题来源和学习要点复核。"}`);
      if (item.sourceIndexes?.length) lines.push(`  来源索引：${item.sourceIndexes.map((index) => index + 1).join("、")}`);
    }
  }
  if (studio.actionPlan?.steps?.length) {
    lines.push("");
    lines.push("### 亚马逊行动实验计划");
    lines.push(`边界：${studio.actionPlan.boundary}`);
    lines.push(`摘要：${studio.actionPlan.summary}`);
    for (const step of studio.actionPlan.steps) {
      lines.push(`- ${step.label}`);
      if (step.purpose) lines.push(`  目的：${step.purpose}`);
      if (step.requiredData?.length) lines.push(`  需要数据：${step.requiredData.join("、")}`);
      if (step.successSignal) lines.push(`  判断信号：${step.successSignal}`);
      if (step.sourceIndexes?.length) lines.push(`  来源索引：${step.sourceIndexes.map((index) => index + 1).join("、")}`);
    }
  }
  if (studio.mindMap?.nodes?.length) {
    lines.push("");
    lines.push("### 思维导图数据");
    lines.push(`- 节点：${studio.mindMap.nodes.length}`);
    lines.push(`- 连线：${studio.mindMap.edges?.length || 0}`);
  }
  if (studio.sourceTable?.length) {
    lines.push("");
    lines.push("### 来源数据表");
    for (const source of studio.sourceTable.slice(0, 12)) {
      lines.push(`- ${source.index}. ${source.author} · ${source.title} · ${source.identity}`);
    }
  }
  return `${lines.join("\n").trim()}\n`;
}

function csvRows(headers, rows) {
  return [
    headers.map(csvCell).join(","),
    ...rows.map((row) => row.map(csvCell).join(",")),
  ].join("\n") + "\n";
}

function csvCell(value) {
  const text = String(value ?? "").replace(/\r?\n/g, " ").trim();
  return `"${text.replace(/"/g, '""')}"`;
}

function notebookTitleFromMessages(messages = []) {
  const firstQuestion = messages.find((message) => message?.role === "user" && String(message.content || "").trim());
  return compactServerText(firstQuestion?.content || "亚马逊学习专题", 80);
}

function notebookTopicFromMessages(messages = []) {
  const userMessages = messages
    .filter((message) => message?.role === "user" && String(message.content || "").trim())
    .slice(0, 3)
    .map((message) => compactServerText(message.content, 80));
  return userMessages.join(" / ") || "亚马逊学习专题";
}

function compactServerText(value, max = 240) {
  return String(value || "").replace(/\s+/g, " ").trim().slice(0, max);
}

function normalizeWorkflowIntent(intent) {
  if (!intent || typeof intent !== "object") return undefined;
  const text = (value, maxLength) => (typeof value === "string" ? value.slice(0, maxLength) : "");
  const type = text(intent.type || "method_learning", 60);
  const label = text(intent.label || "方法学习", 80);
  if (!type && !label) return undefined;
  return {
    type,
    label,
    goal: text(intent.goal, 220),
    primaryAction: text(intent.primaryAction, 220),
    nextPrompt: text(intent.nextPrompt, 420),
    boundary: text(intent.boundary, 260),
    confidence: text(intent.confidence || "medium", 20),
  };
}

function normalizeIntentPreference(value) {
  const raw = typeof value === "object" && value ? value.type : value;
  const type = String(raw || "").trim();
  return ["method_learning", "product_diagnosis", "experiment_review", "answer_retry", "source_search"].includes(type)
    ? type
    : "";
}

function normalizeLearningQueue(queue) {
  if (!queue || typeof queue !== "object") return undefined;
  const safeText = (value, max = 240) => String(value || "").slice(0, max);
  const items = Array.isArray(queue.items)
    ? queue.items
        .map((item, index) => ({
          id: safeText(item?.id || `queue:${index}`, 80),
          kind: safeText(item?.kind || "task", 40),
          label: safeText(item?.label, 120),
          reason: safeText(item?.reason, 240),
          action: safeText(item?.action, 40),
          actionLabel: safeText(item?.actionLabel || "继续", 60),
          completionMode: safeText(item?.completionMode || "manual", 40),
          requiresEvidenceGate: item?.requiresEvidenceGate === true,
          locked: item?.locked === true,
          lockedLabel: safeText(item?.lockedLabel || "", 80) || undefined,
          lockedReason: safeText(item?.lockedReason || "", 220) || undefined,
          boundary: safeText(item?.boundary, 220),
          sourceIndex: Number.isInteger(item?.sourceIndex) ? item.sourceIndex : undefined,
          claimId: typeof item?.claimId === "string" ? safeText(item.claimId, 80) : undefined,
          prompt: typeof item?.prompt === "string" ? safeText(item.prompt, 420) : undefined,
          section: typeof item?.section === "string" ? safeText(item.section, 40) : undefined,
          done: item?.done === true,
        }))
        .filter((item) => item.id && item.label)
        .slice(0, 6)
    : [];
  if (items.length === 0) return undefined;
  const completed = items.filter((item) => item.done).length;
  const total = items.length;
  const currentItem = items.find((item) => !item.done) || items[0];
  return {
    summary: safeText(queue.summary || "把本轮回答推进成连续学习动作。", 260),
    boundary: safeText(queue.boundary || "学习队列只记录处理动作，不代表结论已经被证明。", 260),
    currentItemId: currentItem?.id || "",
    progress: {
      completed,
      total,
      percent: total > 0 ? Math.round((completed / total) * 100) : 0,
    },
    items,
  };
}

function normalizeLearningMemoryReminderForMessage(reminder) {
  if (!reminder || typeof reminder !== "object") return undefined;
  const items = Array.isArray(reminder.items)
    ? reminder.items.slice(0, 3).map((item) => ({
        id: typeof item?.id === "string" ? item.id.slice(0, 120) : undefined,
        title: typeof item?.title === "string" ? item.title.slice(0, 120) : "",
        excerpt: typeof item?.excerpt === "string" ? item.excerpt.slice(0, 420) : "",
        namespace: typeof item?.namespace === "string" ? item.namespace.slice(0, 120) : "",
        key: typeof item?.key === "string" ? item.key.slice(0, 180) : "",
        documentId: typeof item?.documentId === "string" ? item.documentId.slice(0, 120) : "",
        score: Number.isFinite(Number(item?.score)) ? Number(item.score) : undefined,
        vectorSimilarity: Number.isFinite(Number(item?.vectorSimilarity)) ? Number(item.vectorSimilarity) : undefined,
        sourceType: typeof item?.sourceType === "string" ? item.sourceType.slice(0, 120) : "",
      }))
    : [];
  if (items.length === 0) return undefined;
  return {
    label: typeof reminder.label === "string" ? reminder.label.slice(0, 80) : "本地学习档案提醒",
    boundary:
      typeof reminder.boundary === "string"
        ? reminder.boundary.slice(0, 240)
        : "这些是你保存过的学习档案提醒，不是作者原文证据；本轮引用和证据链仍只来自作者资料。",
    alignment: normalizeLearningMemoryAlignmentForMessage(reminder.alignment),
    items,
  };
}

function normalizeLearningMemoryAlignmentForMessage(alignment) {
  if (!alignment || typeof alignment !== "object") return undefined;
  const status = ["aligned", "conflict", "needs_source", "neutral"].includes(alignment.status) ? alignment.status : "neutral";
  return {
    status,
    label: typeof alignment.label === "string" ? alignment.label.slice(0, 100) : "",
    message: typeof alignment.message === "string" ? alignment.message.slice(0, 260) : "",
    conflicts: normalizeLearningAlignmentRows(alignment.conflicts),
    matches: normalizeLearningAlignmentRows(alignment.matches),
  };
}

function normalizeAnswerEffectiveness(effectiveness) {
  if (!effectiveness || typeof effectiveness !== "object") return undefined;
  const status = ["resolved", "needs_source", "switch_intent", "add_product_data"].includes(effectiveness.status)
    ? effectiveness.status
    : "";
  if (!status) return undefined;
  return {
    status,
    updatedAt: typeof effectiveness.updatedAt === "string" ? effectiveness.updatedAt.slice(0, 40) : "",
    question: typeof effectiveness.question === "string" ? effectiveness.question.slice(0, 240) : "",
  };
}

function normalizeUsageFootprint(footprint) {
  if (!footprint || typeof footprint !== "object") return undefined;
  const estimate = footprint.estimate && typeof footprint.estimate === "object" ? footprint.estimate : {};
  return {
    mode: typeof footprint.mode === "string" ? footprint.mode.slice(0, 40) : "local_ollama",
    model: typeof footprint.model === "string" ? footprint.model.slice(0, 100) : "",
    cloudBillableTokens: Number.isFinite(Number(footprint.cloudBillableTokens)) ? Math.max(0, Math.floor(Number(footprint.cloudBillableTokens))) : 0,
    cloudBillableCostText: typeof footprint.cloudBillableCostText === "string" ? footprint.cloudBillableCostText.slice(0, 180) : "",
    summary: typeof footprint.summary === "string" ? footprint.summary.slice(0, 240) : "",
    boundary: typeof footprint.boundary === "string" ? footprint.boundary.slice(0, 260) : "",
    estimate: {
      questionTokens: Number.isFinite(Number(estimate.questionTokens)) ? Math.max(0, Math.floor(Number(estimate.questionTokens))) : 0,
      retrievalTokens: Number.isFinite(Number(estimate.retrievalTokens)) ? Math.max(0, Math.floor(Number(estimate.retrievalTokens))) : 0,
      sourceTokens: Number.isFinite(Number(estimate.sourceTokens)) ? Math.max(0, Math.floor(Number(estimate.sourceTokens))) : 0,
      answerTokens: Number.isFinite(Number(estimate.answerTokens)) ? Math.max(0, Math.floor(Number(estimate.answerTokens))) : 0,
      totalCloudEquivalentTokens: Number.isFinite(Number(estimate.totalCloudEquivalentTokens))
        ? Math.max(0, Math.floor(Number(estimate.totalCloudEquivalentTokens)))
        : 0,
    },
  };
}

function normalizeLearningAlignmentRows(rows) {
  return Array.isArray(rows)
    ? rows
        .slice(0, 4)
        .map((item) => ({
          concept: typeof item?.concept === "string" ? item.concept.slice(0, 40) : "",
          memoryTitle: typeof item?.memoryTitle === "string" ? item.memoryTitle.slice(0, 120) : "",
          memoryExcerpt: typeof item?.memoryExcerpt === "string" ? item.memoryExcerpt.slice(0, 260) : "",
          memoryStance: typeof item?.memoryStance === "string" ? item.memoryStance.slice(0, 40) : "",
          sourceStance: typeof item?.sourceStance === "string" ? item.sourceStance.slice(0, 40) : "",
          sourceIndex: Number.isInteger(item?.sourceIndex) ? item.sourceIndex : undefined,
          sourceTitle: typeof item?.sourceTitle === "string" ? item.sourceTitle.slice(0, 120) : "",
          author: typeof item?.author === "string" ? item.author.slice(0, 80) : "",
          quote: typeof item?.quote === "string" ? item.quote.slice(0, 180) : "",
        }))
        .filter((item) => item.concept)
    : [];
}

function normalizeOpenHumanMemoryForMessage(memory) {
  if (!memory || typeof memory !== "object") return undefined;
  const status = ["not_synced", "pending", "synced", "failed", "skipped"].includes(memory.status) ? memory.status : "not_synced";
  return {
    status,
    namespace: String(memory.namespace || "").slice(0, 80),
    key: String(memory.key || "").slice(0, 180),
    documentId: String(memory.documentId || memory.document_id || "").slice(0, 120),
    documentTitle: String(memory.documentTitle || memory.document_title || "").slice(0, 180),
    syncedAt: String(memory.syncedAt || memory.synced_at || "").slice(0, 40),
    error: String(memory.error || "").slice(0, 220),
    indexStatus: String(memory.indexStatus || memory.index_status || "").slice(0, 40),
    totalChunks: Number.isFinite(Number(memory.totalChunks ?? memory.total_chunks)) ? Math.max(0, Number(memory.totalChunks ?? memory.total_chunks)) : 0,
    indexedChunks: Number.isFinite(Number(memory.indexedChunks ?? memory.indexed_chunks)) ? Math.max(0, Number(memory.indexedChunks ?? memory.indexed_chunks)) : 0,
  };
}

function normalizeGraph(graph) {
  if (!graph || typeof graph !== "object") return undefined;
  const nodes = Array.isArray(graph.nodes) ? graph.nodes : [];
  const edges = Array.isArray(graph.edges) ? graph.edges : [];
  return {
    nodes: nodes
      .filter((node) => node && typeof node.id === "string" && typeof node.label === "string")
      .slice(0, 32),
    edges: edges
      .filter((edge) => edge && typeof edge.from === "string" && typeof edge.to === "string")
      .slice(0, 64),
  };
}

function normalizeKnowledgeGapRadar(radar) {
  if (!radar || typeof radar !== "object") return undefined;
  const safeText = (value, max = 240) => String(value || "").slice(0, max);
  const safeInts = (items, max = 8) => Array.isArray(items) ? items.filter(Number.isInteger).slice(0, max) : [];
  const safeIds = (items, max = 8) => Array.isArray(items) ? items.map((id) => safeText(id, 80)).filter(Boolean).slice(0, max) : [];
  const normalizeGap = (item, index) => {
    if (!item || typeof item !== "object") return null;
    return {
      id: safeText(item.id || `gap:${index}`, 80),
      kind: safeText(item.kind || "gap", 40),
      label: safeText(item.label, 90),
      reason: safeText(item.reason, 240),
      prompt: safeText(item.prompt, 520),
      sourceIndexes: safeInts(item.sourceIndexes),
      claimIds: safeIds(item.claimIds),
      canUseAsEvidence: item.canUseAsEvidence === true,
    };
  };
  const gaps = Array.isArray(radar.gaps)
    ? radar.gaps.map(normalizeGap).filter((item) => item?.id && item.label).slice(0, 6)
    : [];
  const priority = normalizeGap(radar.priority, 0) || gaps[0];
  const metrics = radar.metrics && typeof radar.metrics === "object" ? radar.metrics : {};
  if (gaps.length === 0 && !priority) return undefined;
  return {
    title: safeText(radar.title || "知识缺口雷达", 100),
    status: ["needs_source", "needs_review", "needs_data", "ready_to_validate"].includes(radar.status)
      ? radar.status
      : "needs_review",
    summary: safeText(radar.summary, 320),
    priority,
    gaps,
    metrics: {
      sourceCount: Number(metrics.sourceCount || 0),
      evidenceCount: Number(metrics.evidenceCount || 0),
      authorCount: Number(metrics.authorCount || 0),
      conflictCount: Number(metrics.conflictCount || 0),
      missingDataCount: Number(metrics.missingDataCount || 0),
      unsupportedCount: Number(metrics.unsupportedCount || 0),
    },
    boundary: safeText(
      radar.boundary || "知识缺口雷达只决定下一步补资料、补数据或复核路径，不改变作者原文证据边界。",
      420,
    ),
  };
}

function normalizeNextBestSource(route) {
  if (!route || typeof route !== "object") return undefined;
  const safeText = (value, max = 240) => String(value || "").slice(0, max);
  const safeInts = (items, max = 6) => Array.isArray(items) ? items.filter(Number.isInteger).slice(0, max) : [];
  const safeIds = (items, max = 6) => Array.isArray(items) ? items.map((id) => safeText(id, 80)).filter(Boolean).slice(0, max) : [];
  const normalizeItem = (item, index, prefix = "next-source") => {
    if (!item || typeof item !== "object") return null;
    return {
      id: safeText(item.id || `${prefix}:${index}`, 100),
      kind: safeText(item.kind || "source", 40),
      label: safeText(item.label, 140),
      author: safeText(item.author, 80),
      title: safeText(item.title, 180),
      sourceIndex: Number.isInteger(item.sourceIndex) ? item.sourceIndex : undefined,
      claimId: safeText(item.claimId, 80),
      quote: safeText(item.quote, 420),
      reason: safeText(item.reason, 260),
      prompt: safeText(item.prompt, 560),
      sourceIndexes: safeInts(item.sourceIndexes),
      claimIds: safeIds(item.claimIds),
      canUseAsEvidence: item.canUseAsEvidence === true,
      sourceCanUseAsEvidence: item.sourceCanUseAsEvidence === true,
    };
  };
  const recommended = normalizeItem(route.recommended, 0);
  if (!recommended || !recommended.label) return undefined;
  return {
    title: safeText(route.title || "下一步资料选择", 100),
    status: safeText(route.status || "needs_review", 40),
    summary: safeText(route.summary, 320),
    topic: safeText(route.topic, 140),
    criteria: Array.isArray(route.criteria)
      ? route.criteria.map((item, index) => ({
          id: safeText(item?.id || `criterion:${index}`, 80),
          label: safeText(item?.label, 90),
          status: safeText(item?.status, 40),
          detail: safeText(item?.detail, 260),
        })).filter((item) => item.label).slice(0, 6)
      : [],
    recommended,
    alternatives: Array.isArray(route.alternatives)
      ? route.alternatives.map((item, index) => normalizeItem(item, index, "next-alt")).filter((item) => item?.label).slice(0, 4)
      : [],
    boundary: safeText(
      route.boundary || "下一步资料选择只安排阅读、复核和补材料顺序；推荐理由是系统整理，不是新的作者原文证据。",
      420,
    ),
  };
}

function normalizeTopicSourceTree(tree) {
  if (!tree || typeof tree !== "object") return undefined;
  const safeText = (value, max = 240) => String(value || "").slice(0, max);
  const status = ["ready", "needs_source"].includes(tree.status) ? tree.status : "needs_source";
  return {
    title: safeText(tree.title || "本轮主题来源树", 100),
    status,
    summary: safeText(tree.summary, 320),
    boundary: safeText(tree.boundary, 420),
    topic: tree.topic && typeof tree.topic === "object"
      ? {
          id: safeText(tree.topic.id || "topic:question", 80),
          label: safeText(tree.topic.label, 140),
          question: safeText(tree.topic.question, 220),
          retrievalQuestion: safeText(tree.topic.retrievalQuestion, 320),
        }
      : undefined,
    concepts: Array.isArray(tree.concepts)
      ? tree.concepts
          .map((item, index) => ({
            id: safeText(item?.id || `topic-concept:${index}`, 80),
            label: safeText(item?.label, 60),
            identity: safeText(item?.identity || "系统整理", 40),
            canUseAsEvidence: item?.canUseAsEvidence === true,
            sourceIndexes: Array.isArray(item?.sourceIndexes)
              ? item.sourceIndexes.filter((sourceIndex) => Number.isInteger(sourceIndex)).slice(0, 5)
              : [],
            prompt: safeText(item?.prompt, 520),
          }))
          .filter((item) => item.label)
          .slice(0, 8)
      : [],
    sources: Array.isArray(tree.sources)
      ? tree.sources
          .map((item, index) => ({
            id: safeText(item?.id || `topic-source:${index}`, 80),
            claimId: safeText(item?.claimId, 80),
            sourceIndex: Number.isInteger(item?.sourceIndex) ? item.sourceIndex : undefined,
            identity: safeText(item?.identity || "作者原文", 40),
            canUseAsEvidence: item?.canUseAsEvidence === true,
            author: safeText(item?.author, 80),
            date: safeText(item?.date, 32),
            title: safeText(item?.title, 180),
            sourceUrl: safeText(item?.sourceUrl, 260),
            sourcePath: safeText(item?.sourcePath, 260),
            label: safeText(item?.label, 120),
            quote: safeText(item?.quote, 500),
            reason: safeText(item?.reason, 240),
            concepts: Array.isArray(item?.concepts) ? item.concepts.map((concept) => safeText(concept, 50)).filter(Boolean).slice(0, 5) : [],
            prompt: safeText(item?.prompt, 560),
          }))
          .filter((item) => Number.isInteger(item.sourceIndex))
          .slice(0, 5)
      : [],
    authors: Array.isArray(tree.authors)
      ? tree.authors
          .map((item, index) => ({
            id: safeText(item?.id || `topic-author:${index}`, 80),
            author: safeText(item?.author, 80),
            role: safeText(item?.role, 100),
            sourceIndexes: Array.isArray(item?.sourceIndexes)
              ? item.sourceIndexes.filter((sourceIndex) => Number.isInteger(sourceIndex)).slice(0, 5)
              : [],
            sourceCount: Number.isFinite(Number(item?.sourceCount)) ? Math.max(0, Number(item.sourceCount)) : 0,
            concepts: Array.isArray(item?.concepts) ? item.concepts.map((concept) => safeText(concept, 50)).filter(Boolean).slice(0, 5) : [],
            summary: safeText(item?.summary, 280),
          }))
          .filter((item) => item.author)
          .slice(0, 4)
      : [],
    paths: Array.isArray(tree.paths)
      ? tree.paths
          .map((item, index) => ({
            id: safeText(item?.id || `topic-path:${index}`, 80),
            kind: safeText(item?.kind, 40),
            label: safeText(item?.label, 120),
            detail: safeText(item?.detail, 300),
            sourceIndex: Number.isInteger(item?.sourceIndex) ? item.sourceIndex : undefined,
            prompt: safeText(item?.prompt, 560),
          }))
          .filter((item) => item.label || item.detail)
          .slice(0, 6)
      : [],
    nextPrompts: Array.isArray(tree.nextPrompts) ? tree.nextPrompts.map((item) => safeText(item, 560)).filter(Boolean).slice(0, 3) : [],
  };
}

function normalizeRankedEvidence(items) {
  if (!Array.isArray(items)) return undefined;
  return items
    .map((item) => ({
      evidenceIndex: Number.isInteger(item?.evidenceIndex) ? item.evidenceIndex : undefined,
      sourceIndex: Number.isInteger(item?.sourceIndex) ? item.sourceIndex : 0,
      quote: String(item?.quote || "").slice(0, 700),
      score: Number(item?.score || 0),
      author: String(item?.author || "").slice(0, 80),
      title: String(item?.title || "").slice(0, 180),
      date: String(item?.date || "").slice(0, 32),
    }))
    .filter((item) => item.quote)
    .slice(0, 8);
}

function normalizeProductInputForAsk(input) {
  if (!input || typeof input !== "object") return undefined;
  const text = String(input.text || input.rawText || "").slice(0, 3000);
  const rawIntake = input.intake && typeof input.intake === "object" ? input.intake : input;
  const sections = Array.isArray(rawIntake.sections)
    ? rawIntake.sections
        .map((section) => ({
          id: String(section?.id || "").slice(0, 40),
          label: String(section?.label || "").slice(0, 80),
          items: Array.isArray(section?.items)
            ? section.items.map((item) => String(item || "").slice(0, 220)).filter(Boolean).slice(0, 8)
            : [],
          missing: Array.isArray(section?.missing)
            ? section.missing.map((item) => String(item || "").slice(0, 120)).filter(Boolean).slice(0, 4)
            : [],
        }))
        .filter((section) => section.label || section.items.length > 0)
        .slice(0, 8)
    : [];
  const missing = Array.isArray(rawIntake.missing)
    ? rawIntake.missing.map((item) => String(item || "").slice(0, 120)).filter(Boolean).slice(0, 10)
    : [];
  if (!text && sections.length === 0 && missing.length === 0) return undefined;
  return {
    text,
    intake: {
      summary: String(rawIntake.summary || "").slice(0, 180),
      sections,
      missing,
      diagnosticPrompt: String(rawIntake.diagnosticPrompt || "").slice(0, 1800),
      caution: String(rawIntake.caution || "").slice(0, 180),
    },
  };
}

function normalizeProductInputSummary(summary) {
  if (!summary || typeof summary !== "object") return undefined;
  const facts = Array.isArray(summary.facts)
    ? summary.facts
        .map((section) => ({
          id: String(section?.id || "").slice(0, 40),
          label: String(section?.label || "").slice(0, 80),
          items: Array.isArray(section?.items)
            ? section.items.map((item) => String(item || "").slice(0, 220)).filter(Boolean).slice(0, 6)
            : [],
          missing: Array.isArray(section?.missing)
            ? section.missing.map((item) => String(item || "").slice(0, 120)).filter(Boolean).slice(0, 4)
            : [],
        }))
        .filter((section) => section.items.length > 0)
        .slice(0, 8)
    : [];
  const missing = Array.isArray(summary.missing)
    ? summary.missing.map((item) => String(item || "").slice(0, 120)).filter(Boolean).slice(0, 8)
    : [];
  if (facts.length === 0 && missing.length === 0 && !summary.summary) return undefined;
  return {
    source: "user_input",
    summary: String(summary.summary || "").slice(0, 180),
    facts,
    missing,
    caution: String(summary.caution || "").slice(0, 220),
  };
}

function normalizeDiagnosisPanel(panel) {
  if (!panel || typeof panel !== "object") return undefined;
  const tracks = Array.isArray(panel.tracks)
    ? panel.tracks
        .map((track, trackIndex) => ({
          id: String(track?.id || `track:${trackIndex}`).slice(0, 80),
          label: String(track?.label || "").slice(0, 80),
          level: String(track?.level || "").slice(0, 40),
          why: String(track?.why || "").slice(0, 220),
          prompt: String(track?.prompt || "").slice(0, 420),
          checks: Array.isArray(track?.checks)
            ? track.checks
                .map((check, checkIndex) => ({
                  id: String(check?.id || `${track?.id || "track"}:${checkIndex}`).slice(0, 100),
                  label: String(check?.label || check || "").slice(0, 160),
                }))
                .filter((check) => check.label)
                .slice(0, 6)
            : [],
        }))
        .filter((track) => track.label && track.checks.length > 0)
        .slice(0, 6)
    : [];
  const checked = panel.checked && typeof panel.checked === "object"
    ? Object.fromEntries(
        Object.entries(panel.checked)
          .filter(([key, value]) => typeof key === "string" && value === true)
          .slice(0, 80)
          .map(([key]) => [key.slice(0, 120), true]),
      )
    : undefined;
  if (tracks.length === 0) return undefined;
  return {
    summary: String(panel.summary || "").slice(0, 220),
    priority: String(panel.priority || "").slice(0, 180),
    reason: String(panel.reason || "").slice(0, 280),
    tracks,
    checked,
    caution: String(panel.caution || "").slice(0, 260),
  };
}

function normalizeValidationPack(pack) {
  if (!pack || typeof pack !== "object") return undefined;
  const safeText = (value, max = 240) => String(value || "").slice(0, max);
  const status = pack.status === "source_backed" ? "source_backed" : "needs_source";
  const hypotheses = Array.isArray(pack.hypotheses)
    ? pack.hypotheses
        .map((item, index) => ({
          id: safeText(item?.id || `hypothesis:${index}`, 80),
          label: safeText(item?.label, 160),
          sourceIndex: Number.isInteger(item?.sourceIndex) ? item.sourceIndex : undefined,
          author: safeText(item?.author, 80),
          sourceTitle: safeText(item?.sourceTitle, 180),
          quote: safeText(item?.quote, 260),
          verifyWith: safeText(item?.verifyWith, 220),
        }))
        .filter((item) => item.label)
        .slice(0, 4)
    : [];
  const dataRequests = Array.isArray(pack.dataRequests)
    ? pack.dataRequests
        .map((item, index) => ({
          id: safeText(item?.id || `data:${index}`, 80),
          label: safeText(item?.label, 120),
          why: safeText(item?.why, 220),
          placeholder: safeText(item?.placeholder, 180),
        }))
        .filter((item) => item.label)
        .slice(0, 8)
    : [];
  const experiments = Array.isArray(pack.experiments)
    ? pack.experiments
        .map((item, index) => ({
          id: safeText(item?.id || `experiment:${index}`, 80),
          title: safeText(item?.title, 120),
          steps: Array.isArray(item?.steps) ? item.steps.map((step) => safeText(step, 160)).filter(Boolean).slice(0, 5) : [],
          successSignal: safeText(item?.successSignal, 220),
        }))
        .filter((item) => item.title)
        .slice(0, 3)
    : [];
  const decisionRules = Array.isArray(pack.decisionRules)
    ? pack.decisionRules
        .map((item) => ({
          if: safeText(item?.if, 180),
          then: safeText(item?.then, 220),
        }))
        .filter((item) => item.if || item.then)
        .slice(0, 5)
    : [];
  if (dataRequests.length === 0 && hypotheses.length === 0 && experiments.length === 0 && decisionRules.length === 0) return undefined;
  return {
    title: safeText(pack.title || "本轮业务验证任务包", 100),
    status,
    summary: safeText(pack.summary, 260),
    boundary: safeText(pack.boundary, 280),
    hypotheses,
    dataRequests,
    experiments,
    decisionRules,
    businessDecision: normalizeBusinessDecision(pack.businessDecision),
    followUpPrompt: safeText(pack.followUpPrompt, 520),
  };
}

function normalizeBusinessDecision(decision) {
  if (!decision || typeof decision !== "object") return undefined;
  const safeText = (value, max = 240) => String(value || "").slice(0, max);
  const rows = (items, limit) => Array.isArray(items)
    ? items
        .map((item, index) => ({
          id: safeText(item?.id || `business-data:${index}`, 80),
          label: safeText(item?.label, 120),
          value: safeText(item?.value, 180),
          why: safeText(item?.why, 220),
          role: safeText(item?.role, 40),
        }))
        .filter((item) => item.label)
        .slice(0, limit)
    : [];
  return {
    title: safeText(decision.title || "当前产品判断", 100),
    status: ["ready", "insufficient_data", "needs_source", "needs_data"].includes(decision.status) ? decision.status : "needs_data",
    priority: safeText(decision.priority || "insufficient", 80),
    label: safeText(decision.label, 220),
    summary: safeText(decision.summary, 320),
    supportingData: rows(decision.supportingData, 6),
    opposingData: rows(decision.opposingData, 4),
    missingData: rows(decision.missingData, 5),
    boundary: safeText(decision.boundary || "用户产品数据不是作者原文证据，只用于判断适配性。", 320),
  };
}

function normalizeEvidenceChain(chain) {
  if (!chain || typeof chain !== "object") return undefined;
  const claims = Array.isArray(chain.claims) ? chain.claims : [];
  return {
    summary: String(chain.summary || "").slice(0, 160),
    claims: claims
      .map((claim, index) => ({
        id: String(claim?.id || `claim:${index}`).slice(0, 80),
        type: String(claim?.type || "system_inference").slice(0, 40),
        label: String(claim?.label || "").slice(0, 60),
        canProve: claim?.canProve === true,
        canUseAsEvidence: claim?.canUseAsEvidence === true,
        evidenceKind: String(claim?.evidenceKind || "").slice(0, 40),
        trustKind: String(claim?.trustKind || "").slice(0, 40),
        trustLabel: String(claim?.trustLabel || "").slice(0, 80),
        trustLevel: String(claim?.trustLevel || "").slice(0, 40),
        text: String(claim?.text || "").slice(0, 500),
        quote: claim?.quote ? String(claim.quote).slice(0, 700) : undefined,
        sourceIndex: Number.isInteger(claim?.sourceIndex) ? claim.sourceIndex : undefined,
        evidenceIndexes: Array.isArray(claim?.evidenceIndexes)
          ? claim.evidenceIndexes.filter(Number.isInteger).slice(0, 4)
          : undefined,
        author: String(claim?.author || "").slice(0, 80),
        title: String(claim?.title || "").slice(0, 180),
        date: String(claim?.date || "").slice(0, 32),
        basis: String(claim?.basis || "").slice(0, 220),
        validation: String(claim?.validation || "").slice(0, 220),
      }))
      .filter((claim) => claim.text)
      .slice(0, 12),
  };
}

function normalizeEvidenceAudit(audit) {
  if (!audit || typeof audit !== "object") return undefined;
  const checks = Array.isArray(audit.checks) ? audit.checks : [];
  const counts = audit.counts && typeof audit.counts === "object" ? audit.counts : {};
  const feedback = ["useful", "citation_wrong", "retry"].includes(audit.feedback) ? audit.feedback : "";
  const label = String(audit.label || "").replace("可信度较高", "引用支撑较充分");
  return {
    level: ["high", "medium", "low"].includes(audit.level) ? audit.level : "low",
    label: label.slice(0, 40),
    summary: String(audit.summary || "").slice(0, 260),
    counts: {
      sources: Number(counts.sources || 0),
      sourceEvidence: Number(counts.sourceEvidence || 0),
      systemInferences: Number(counts.systemInferences || 0),
      actionAdvice: Number(counts.actionAdvice || 0),
      needsSource: Number(counts.needsSource || 0),
    },
    checks: checks
      .map((check) => ({
        id: String(check?.id || "").slice(0, 80),
        label: String(check?.label || "").slice(0, 80),
        status: String(check?.status || "warn").slice(0, 20),
        message: String(check?.message || "").slice(0, 260),
        sourceIndexes: Array.isArray(check?.sourceIndexes)
          ? check.sourceIndexes.filter(Number.isInteger).slice(0, 8)
          : undefined,
      }))
      .filter((check) => check.id && check.label && check.message)
      .slice(0, 6),
    conflictSignals: Array.isArray(audit.conflictSignals)
      ? audit.conflictSignals
          .map((item) => ({
            concept: String(item?.concept || "").slice(0, 40),
            relatedConcepts: Array.isArray(item?.relatedConcepts) ? item.relatedConcepts.map((concept) => String(concept || "").slice(0, 40)).filter(Boolean).slice(0, 6) : [],
            role: String(item?.role || "primary").slice(0, 40),
            supportingReasons: normalizeSupportingConflictReasons(item?.supportingReasons),
            sourceIndexes: Array.isArray(item?.sourceIndexes) ? item.sourceIndexes.filter(Number.isInteger).slice(0, 8) : [],
            support: normalizeConflictSide(item?.support),
            caution: normalizeConflictSide(item?.caution),
            comparison: normalizeConflictComparison(item?.comparison),
          }))
          .filter((item) => item.concept)
          .slice(0, 4)
      : [],
    caution: String(audit.caution || "").replace("可信度检查", "引用核对").slice(0, 260),
    feedback,
  };
}

function normalizeSourceTrust(trust) {
  if (!trust || typeof trust !== "object") return undefined;
  const categories = Array.isArray(trust.categories) ? trust.categories : [];
  const title = String(trust.title || "本轮来源核对状态").replace("本轮来源可信链路", "本轮来源核对状态");
  return {
    title: title.slice(0, 100),
    status: ["source_backed", "needs_source"].includes(trust.status) ? trust.status : "needs_source",
    label: String(trust.label || "").slice(0, 140),
    summary: String(trust.summary || "").slice(0, 260),
    boundary: String(trust.boundary || "").slice(0, 360),
    sourceTree: normalizeSourceTreeTrustForMessage(trust.sourceTree),
    categories: categories
      .map((item, index) => ({
        id: String(item?.id || `trust:${index}`).slice(0, 80),
        label: String(item?.label || "").slice(0, 80),
        status: String(item?.status || "missing").slice(0, 40),
        count: Number.isFinite(Number(item?.count)) ? Math.max(0, Number(item.count)) : 0,
        description: String(item?.description || "").slice(0, 220),
        claimIds: Array.isArray(item?.claimIds) ? item.claimIds.map((id) => String(id || "").slice(0, 80)).filter(Boolean).slice(0, 8) : [],
        sourceIndexes: Array.isArray(item?.sourceIndexes) ? item.sourceIndexes.filter(Number.isInteger).slice(0, 8) : [],
      }))
      .filter((item) => item.id && item.label)
      .slice(0, 8),
  };
}

function normalizeSourceTreeTrustForMessage(route) {
  if (!route || typeof route !== "object") return undefined;
  const status = ["active", "unresolved", "summary_only", "empty"].includes(route.status) ? route.status : "empty";
  return {
    title: String(route.title || "OpenHuman 来源树找路").slice(0, 100),
    status,
    summary: String(route.summary || "").slice(0, 260),
    boundary: String(route.boundary || "").slice(0, 360),
    candidateCount: Number.isFinite(Number(route.candidateCount)) ? Math.max(0, Number(route.candidateCount)) : 0,
    resolvedSourceCount: Number.isFinite(Number(route.resolvedSourceCount)) ? Math.max(0, Number(route.resolvedSourceCount)) : 0,
    summaryHintCount: Number.isFinite(Number(route.summaryHintCount)) ? Math.max(0, Number(route.summaryHintCount)) : 0,
  };
}

function normalizeSourceTreeCalibrationForMessage(calibration) {
  if (!calibration || typeof calibration !== "object") return undefined;
  const safeText = (value, max = 240) => String(value || "").slice(0, max);
  const status = ["active", "unresolved", "summary_only", "empty"].includes(calibration.status) ? calibration.status : "empty";
  return {
    title: safeText(calibration.title || "OpenHuman 来源树辅助检索", 100),
    status,
    terms: Array.isArray(calibration.terms) ? calibration.terms.map((term) => safeText(term, 40)).filter(Boolean).slice(0, 12) : [],
    candidateCount: Number.isFinite(Number(calibration.candidateCount)) ? Math.max(0, Number(calibration.candidateCount)) : 0,
    resolvedSourceCount: Number.isFinite(Number(calibration.resolvedSourceCount)) ? Math.max(0, Number(calibration.resolvedSourceCount)) : 0,
    summaryHintCount: Number.isFinite(Number(calibration.summaryHintCount)) ? Math.max(0, Number(calibration.summaryHintCount)) : 0,
    summary: safeText(calibration.summary, 320),
    boundary: safeText(calibration.boundary, 420),
    candidates: Array.isArray(calibration.candidates)
      ? calibration.candidates.map((item, index) => ({
          id: safeText(item?.id || `source-tree:candidate:${index}`, 80),
          type: safeText(item?.type || "route_hint", 40),
          label: safeText(item?.label, 140),
          owner: safeText(item?.owner, 80),
          sourceId: safeText(item?.sourceId, 260),
          sourceRef: safeText(item?.sourceRef, 320),
          chunkCount: Number.isFinite(Number(item?.chunkCount)) ? Math.max(0, Number(item.chunkCount)) : 0,
          matchedOriginalSource: item?.matchedOriginalSource === true,
          matchedTitle: safeText(item?.matchedTitle, 160),
          canUseAsEvidence: false,
        })).filter((item) => item.label || item.sourceId || item.sourceRef).slice(0, 8)
      : [],
    summaries: Array.isArray(calibration.summaries)
      ? calibration.summaries.map((item, index) => ({
          id: safeText(item?.id || `source-tree:summary:${index}`, 80),
          type: "summary_hint",
          label: safeText(item?.label, 140),
          treeId: safeText(item?.treeId, 120),
          excerpt: safeText(item?.excerpt, 260),
          canUseAsEvidence: false,
        })).filter((item) => item.label || item.excerpt).slice(0, 4)
      : [],
  };
}

function normalizeSynthesisAnswer(synthesis) {
  if (!synthesis || typeof synthesis !== "object") return undefined;
  const safeText = (value, max = 240) => String(value || "").slice(0, max);
  const status = ["source_backed", "needs_source", "needs_review"].includes(synthesis.status)
    ? synthesis.status
    : "needs_source";
  const supportRows = (items, limit = 3) => Array.isArray(items)
    ? items
        .map((item) => ({
          claimId: safeText(item?.claimId, 80),
          sourceIndex: Number.isInteger(item?.sourceIndex) ? item.sourceIndex : undefined,
          identity: safeText(item?.identity || "作者原文", 40),
          evidenceKind: safeText(item?.evidenceKind || "source_evidence", 40),
          author: safeText(item?.author, 80),
          title: safeText(item?.title, 180),
          date: safeText(item?.date, 32),
          quote: safeText(item?.quote, 360),
        }))
        .filter((item) => item.claimId && Number.isInteger(item.sourceIndex))
        .slice(0, limit)
    : [];
  const points = Array.isArray(synthesis.points)
    ? synthesis.points
        .map((point, index) => ({
          id: safeText(point?.id || `synthesis-point:${index}`, 80),
          label: safeText(point?.label, 140),
          text: safeText(point?.text, 260),
          identity: safeText(point?.identity || "系统综合", 40),
          evidenceKind: safeText(point?.evidenceKind || "system_synthesis", 40),
          canUseAsEvidence: point?.canUseAsEvidence === true,
          isInference: point?.isInference !== false,
          confidence: safeText(point?.confidence || "low", 40),
          basis: safeText(point?.basis, 220),
          claimIds: Array.isArray(point?.claimIds) ? point.claimIds.map((id) => safeText(id, 80)).filter(Boolean).slice(0, 5) : [],
          sourceIndexes: Array.isArray(point?.sourceIndexes) ? point.sourceIndexes.filter(Number.isInteger).slice(0, 5) : [],
          support: supportRows(point?.support, 4),
        }))
        .filter((point) => point.text || point.label)
        .slice(0, 5)
    : [];
  const authorPerspectives = Array.isArray(synthesis.authorPerspectives)
    ? synthesis.authorPerspectives
        .map((item, index) => ({
          id: safeText(item?.id || `synthesis-author:${index}`, 80),
          author: safeText(item?.author, 80),
          claimIds: Array.isArray(item?.claimIds) ? item.claimIds.map((id) => safeText(id, 80)).filter(Boolean).slice(0, 5) : [],
          sourceIndexes: Array.isArray(item?.sourceIndexes) ? item.sourceIndexes.filter(Number.isInteger).slice(0, 5) : [],
          summary: safeText(item?.summary, 260),
        }))
        .filter((item) => item.author)
        .slice(0, 4)
    : [];
  const conflicts = Array.isArray(synthesis.conflicts)
    ? synthesis.conflicts
        .map((item, index) => ({
          id: safeText(item?.id || `synthesis-conflict:${index}`, 80),
          concept: safeText(item?.concept, 80),
          message: safeText(item?.message, 260),
          sourceIndexes: Array.isArray(item?.sourceIndexes) ? item.sourceIndexes.filter(Number.isInteger).slice(0, 6) : [],
        }))
        .filter((item) => item.concept || item.message)
        .slice(0, 4)
    : [];
  const gaps = Array.isArray(synthesis.gaps)
    ? synthesis.gaps
        .map((item, index) => ({
          id: safeText(item?.id || `synthesis-gap:${index}`, 80),
          label: safeText(item?.label, 100),
          reason: safeText(item?.reason, 240),
        }))
        .filter((item) => item.label || item.reason)
        .slice(0, 5)
    : [];
  return {
    title: safeText(synthesis.title || "本轮综合答案", 100),
    status,
    summary: safeText(synthesis.summary, 280),
    sourceCoverage: {
      sourceCount: Number.isFinite(Number(synthesis.sourceCoverage?.sourceCount)) ? Number(synthesis.sourceCoverage.sourceCount) : 0,
      evidenceCount: Number.isFinite(Number(synthesis.sourceCoverage?.evidenceCount)) ? Number(synthesis.sourceCoverage.evidenceCount) : 0,
      authorCount: Number.isFinite(Number(synthesis.sourceCoverage?.authorCount)) ? Number(synthesis.sourceCoverage.authorCount) : 0,
      authors: Array.isArray(synthesis.sourceCoverage?.authors) ? synthesis.sourceCoverage.authors.map((author) => safeText(author, 80)).filter(Boolean).slice(0, 8) : [],
    },
    sourceClaimIds: Array.isArray(synthesis.sourceClaimIds) ? synthesis.sourceClaimIds.map((id) => safeText(id, 80)).filter(Boolean).slice(0, 12) : [],
    points,
    authorPerspectives,
    conflicts,
    gaps,
    boundary: safeText(synthesis.boundary, 360),
  };
}

function normalizeNotebookGuide(guide) {
  if (!guide || typeof guide !== "object") return undefined;
  const safeText = (value, max = 240) => String(value || "").slice(0, max);
  const status = guide.status === "source_backed" ? "source_backed" : "needs_source";
  const normalizeLearningRows = (items, prefix, textKey = "text", limit = 6) => Array.isArray(items)
    ? items.map((item, index) => ({
        id: safeText(item?.id || `${prefix}:${index}`, 80),
        label: safeText(item?.label, 120),
        question: safeText(item?.question, 180),
        [textKey]: safeText(item?.[textKey], 320),
        answer: safeText(item?.answer, 360),
        identity: safeText(item?.identity || "系统整理", 40),
        evidenceKind: safeText(item?.evidenceKind || prefix.replace("notebook-", "notebook_"), 40),
        canUseAsEvidence: false,
        claimIds: Array.isArray(item?.claimIds) ? item.claimIds.map((id) => safeText(id, 80)).filter(Boolean).slice(0, 5) : [],
        sourceIndexes: Array.isArray(item?.sourceIndexes) ? item.sourceIndexes.filter(Number.isInteger).slice(0, 5) : [],
        quote: safeText(item?.quote, 360),
        prompt: safeText(item?.prompt, 520),
      })).filter((item) => item.label || item.question || item[textKey] || item.answer).slice(0, limit)
    : [];
  return {
    title: safeText(guide.title || "本轮学习简报", 100),
    status,
    summary: safeText(guide.summary, 300),
    sourceCoverage: {
      sourceCount: Number.isFinite(Number(guide.sourceCoverage?.sourceCount)) ? Math.max(0, Number(guide.sourceCoverage.sourceCount)) : 0,
      evidenceCount: Number.isFinite(Number(guide.sourceCoverage?.evidenceCount)) ? Math.max(0, Number(guide.sourceCoverage.evidenceCount)) : 0,
      authorCount: Number.isFinite(Number(guide.sourceCoverage?.authorCount)) ? Math.max(0, Number(guide.sourceCoverage.authorCount)) : 0,
      authors: Array.isArray(guide.sourceCoverage?.authors) ? guide.sourceCoverage.authors.map((author) => safeText(author, 80)).filter(Boolean).slice(0, 8) : [],
    },
    briefing: normalizeLearningRows(guide.briefing, "notebook-brief", "text", 5),
    faq: normalizeLearningRows(guide.faq, "notebook-faq", "answer", 5),
    quiz: normalizeLearningRows(guide.quiz, "notebook-quiz", "answer", 5),
    glossary: Array.isArray(guide.glossary)
      ? guide.glossary.map((item, index) => ({
          id: safeText(item?.id || `notebook-glossary:${index}`, 80),
          term: safeText(item?.term, 80),
          definition: safeText(item?.definition, 240),
          identity: safeText(item?.identity || "系统整理", 40),
          canUseAsEvidence: false,
          claimIds: Array.isArray(item?.claimIds) ? item.claimIds.map((id) => safeText(id, 80)).filter(Boolean).slice(0, 4) : [],
          sourceIndexes: Array.isArray(item?.sourceIndexes) ? item.sourceIndexes.filter(Number.isInteger).slice(0, 4) : [],
        })).filter((item) => item.term && item.definition).slice(0, 8)
      : [],
    gaps: Array.isArray(guide.gaps)
      ? guide.gaps.map((item, index) => ({
          id: safeText(item?.id || `notebook-gap:${index}`, 80),
          label: safeText(item?.label, 110),
          reason: safeText(item?.reason, 260),
          prompt: safeText(item?.prompt, 420),
        })).filter((item) => item.label || item.reason).slice(0, 5)
      : [],
    nextPrompts: Array.isArray(guide.nextPrompts) ? guide.nextPrompts.map((item) => safeText(item, 420)).filter(Boolean).slice(0, 4) : [],
    boundary: safeText(guide.boundary || "本轮学习简报是系统整理，不是作者原文证据。", 420),
  };
}

function normalizeSourceStudyPack(pack) {
  if (!pack || typeof pack !== "object") return undefined;
  const safeText = (value, max = 240) => String(value || "").slice(0, max);
  const status = ["needs_review", "needs_source", "ready"].includes(pack.status) ? pack.status : "needs_review";
  const sourceCards = Array.isArray(pack.sourceCards)
    ? pack.sourceCards
        .map((item, index) => ({
          id: safeText(item?.id || `study-source:${index}`, 80),
          claimId: safeText(item?.claimId, 80),
          sourceIndex: Number.isInteger(item?.sourceIndex) ? item.sourceIndex : undefined,
          identity: safeText(item?.identity || "作者原文", 40),
          canUseAsEvidence: item?.canUseAsEvidence === true,
          evidenceKind: safeText(item?.evidenceKind || "source_evidence", 40),
          label: safeText(item?.label, 160),
          title: safeText(item?.title, 180),
          author: safeText(item?.author, 80),
          date: safeText(item?.date, 32),
          sourceUrl: safeText(item?.sourceUrl, 260),
          sourcePath: safeText(item?.sourcePath, 260),
          quote: safeText(item?.quote, 500),
          why: safeText(item?.why, 220),
          prompt: safeText(item?.prompt, 520),
        }))
        .filter((item) => item.claimId && Number.isInteger(item.sourceIndex) && item.quote)
        .slice(0, 5)
    : [];
  const concepts = Array.isArray(pack.concepts)
    ? pack.concepts
        .map((item, index) => ({
          id: safeText(item?.id || `study-concept:${index}`, 80),
          label: safeText(item?.label, 50),
          identity: safeText(item?.identity || "系统整理", 40),
          canUseAsEvidence: item?.canUseAsEvidence === true,
          sourceClaimIds: Array.isArray(item?.sourceClaimIds)
            ? item.sourceClaimIds.map((id) => safeText(id, 80)).filter(Boolean).slice(0, 4)
            : [],
          prompt: safeText(item?.prompt, 420),
        }))
        .filter((item) => item.label)
        .slice(0, 6)
    : [];
  const flashcards = Array.isArray(pack.flashcards)
    ? pack.flashcards
        .map((item, index) => ({
          id: safeText(item?.id || `study-flashcard:${index}`, 80),
          claimId: safeText(item?.claimId, 80),
          sourceIndex: Number.isInteger(item?.sourceIndex) ? item.sourceIndex : undefined,
          identity: safeText(item?.identity || "系统整理", 40),
          canUseAsEvidence: item?.canUseAsEvidence === true,
          question: safeText(item?.question, 160),
          answer: safeText(item?.answer, 420),
          boundary: safeText(item?.boundary, 260),
          prompt: safeText(item?.prompt, 520),
        }))
        .filter((item) => item.claimId && Number.isInteger(item.sourceIndex) && item.question && item.answer)
        .slice(0, 4)
    : [];
  const gaps = Array.isArray(pack.gaps)
    ? pack.gaps
        .map((item, index) => ({
          id: safeText(item?.id || `gap:${index}`, 80),
          label: safeText(item?.label, 90),
          reason: safeText(item?.reason, 240),
          prompt: safeText(item?.prompt, 420),
        }))
        .filter((item) => item.label || item.reason)
        .slice(0, 5)
    : [];
  return {
    title: safeText(pack.title || "本轮来源研读包", 100),
    status,
    boundary: safeText(pack.boundary, 320),
    sourceCards,
    concepts,
    flashcards,
    gaps,
    prompts: Array.isArray(pack.prompts) ? pack.prompts.map((item) => safeText(item, 420)).filter(Boolean).slice(0, 4) : [],
  };
}

function normalizeAuthorPerspectiveRoom(room) {
  if (!room || typeof room !== "object") return undefined;
  const safeText = (value, max = 240) => String(value || "").slice(0, max);
  const status = ["ready", "hidden", "needs_source"].includes(room.status) ? room.status : "hidden";
  const authors = Array.isArray(room.authors)
    ? room.authors
        .map((author, index) => ({
          id: safeText(author?.id || `author-perspective:${index}`, 100),
          author: safeText(author?.author, 80),
          role: safeText(author?.role, 80),
          summary: safeText(author?.summary, 220),
          items: Array.isArray(author?.items)
            ? author.items
                .map((item, itemIndex) => ({
                  id: safeText(item?.id || `author-perspective-item:${itemIndex}`, 100),
                  claimId: safeText(item?.claimId, 80),
                  sourceIndex: Number.isInteger(item?.sourceIndex) ? item.sourceIndex : undefined,
                  identity: safeText(item?.identity || "作者原文", 40),
                  canUseAsEvidence: item?.canUseAsEvidence === true,
                  author: safeText(item?.author, 80),
                  title: safeText(item?.title, 180),
                  date: safeText(item?.date, 32),
                  sourceUrl: safeText(item?.sourceUrl, 360),
                  sourcePath: safeText(item?.sourcePath, 360),
                  quote: safeText(item?.quote, 520),
                  stance: safeText(item?.stance, 180),
                  concepts: Array.isArray(item?.concepts) ? item.concepts.map((concept) => safeText(concept, 40)).filter(Boolean).slice(0, 4) : [],
                }))
                .filter((item) => item.claimId && Number.isInteger(item.sourceIndex) && item.quote)
                .slice(0, 3)
            : [],
        }))
        .filter((author) => author.author && author.items.length > 0)
        .slice(0, 3)
    : [];
  return {
    title: safeText(room.title || "跨作者观点对照", 100),
    status,
    trigger: safeText(room.trigger, 60),
    boundary: safeText(room.boundary, 360),
    authors,
    sharedConcepts: Array.isArray(room.sharedConcepts)
      ? room.sharedConcepts
          .map((item) => ({
            label: safeText(item?.label || item, 60),
            identity: safeText(item?.identity || "系统整理", 40),
            canUseAsEvidence: item?.canUseAsEvidence === true,
          }))
          .filter((item) => item.label)
          .slice(0, 6)
      : [],
    differences: Array.isArray(room.differences)
      ? room.differences
          .map((item, index) => ({
            id: safeText(item?.id || `difference:${index}`, 80),
            label: safeText(item?.label, 140),
            focus: safeText(item?.focus, 220),
            identity: safeText(item?.identity || "系统整理", 40),
            canUseAsEvidence: item?.canUseAsEvidence === true,
          }))
          .filter((item) => item.label || item.focus)
          .slice(0, 3)
      : [],
    requiredData: Array.isArray(room.requiredData)
      ? room.requiredData
          .map((item, index) => ({
            id: safeText(item?.id || `data:${index}`, 80),
            label: safeText(item?.label, 90),
            why: safeText(item?.why, 160),
            placeholder: safeText(item?.placeholder, 120),
          }))
          .filter((item) => item.label)
          .slice(0, 5)
      : [],
    nextPrompt: safeText(room.nextPrompt, 800),
  };
}

function normalizeConflictSide(items) {
  if (!Array.isArray(items)) return [];
  return items
    .map((item) => ({
      sourceIndex: Number.isInteger(item?.sourceIndex) ? item.sourceIndex : undefined,
      title: String(item?.title || "").slice(0, 120),
      author: String(item?.author || "").slice(0, 80),
      date: String(item?.date || "").slice(0, 32),
      sourceUrl: String(item?.sourceUrl || "").slice(0, 260),
      sourcePath: String(item?.sourcePath || "").slice(0, 260),
      quote: String(item?.quote || "").slice(0, 180),
    }))
    .filter((item) => item.quote)
    .slice(0, 2);
}

function normalizeSupportingConflictReasons(items) {
  if (!Array.isArray(items)) return [];
  return items
    .map((item) => ({
      concept: String(item?.concept || "").slice(0, 40),
      relatedConcepts: Array.isArray(item?.relatedConcepts) ? item.relatedConcepts.map((concept) => String(concept || "").slice(0, 40)).filter(Boolean).slice(0, 6) : [],
      sourceIndexes: Array.isArray(item?.sourceIndexes) ? item.sourceIndexes.filter(Number.isInteger).slice(0, 8) : [],
      summary: String(item?.summary || "").slice(0, 220),
      suggestedCheck: String(item?.suggestedCheck || "").slice(0, 260),
    }))
    .filter((item) => item.concept)
    .slice(0, 6);
}

function normalizeConflictComparison(value) {
  if (!value || typeof value !== "object") return undefined;
  const source = (item) => item && typeof item === "object"
    ? {
        sourceIndex: Number.isInteger(item.sourceIndex) ? item.sourceIndex : undefined,
        title: String(item.title || "").slice(0, 120),
        author: String(item.author || "").slice(0, 80),
        date: String(item.date || "").slice(0, 32),
        sourceUrl: String(item.sourceUrl || "").slice(0, 260),
        sourcePath: String(item.sourcePath || "").slice(0, 260),
      }
    : undefined;
  return {
    summary: String(value.summary || "").slice(0, 220),
    differenceFocus: String(value.differenceFocus || "").slice(0, 220),
    supportLabel: String(value.supportLabel || "").slice(0, 60),
    supportQuote: String(value.supportQuote || "").slice(0, 220),
    supportSource: source(value.supportSource),
    cautionLabel: String(value.cautionLabel || "").slice(0, 60),
    cautionQuote: String(value.cautionQuote || "").slice(0, 220),
    cautionSource: source(value.cautionSource),
    suggestedCheck: String(value.suggestedCheck || "").slice(0, 260),
    nextQuestion: normalizeConflictNextQuestion(value.nextQuestion),
  };
}

function normalizeConflictNextQuestion(value) {
  if (!value || typeof value !== "object") return undefined;
  const safeText = (item, max = 260) => String(item || "").slice(0, max);
  const requiredData = Array.isArray(value.requiredData)
    ? value.requiredData
        .map((item) => ({
          id: safeText(item?.id, 40),
          label: safeText(item?.label, 80),
          reason: safeText(item?.reason, 140),
          targetRole: safeText(item?.targetRole || "primary", 40),
          verifies: safeText(item?.verifies, 220),
        }))
        .filter((item) => item.label)
        .slice(0, 8)
    : [];
  const question = safeText(value.question, 1400);
  if (!question && requiredData.length === 0) return undefined;
  return {
    intent: safeText(value.intent || "resolve_conflict", 60),
    question,
    requiredData,
    evidenceRefs: {
      supportSourceIndex: Number.isInteger(value.evidenceRefs?.supportSourceIndex) ? value.evidenceRefs.supportSourceIndex : undefined,
      cautionSourceIndex: Number.isInteger(value.evidenceRefs?.cautionSourceIndex) ? value.evidenceRefs.cautionSourceIndex : undefined,
    },
    boundary: safeText(value.boundary, 220),
  };
}

function normalizeEvidenceFeedback(feedback) {
  if (!feedback || typeof feedback !== "object") return undefined;
  const entries = Object.entries(feedback)
    .filter(([key, value]) => typeof key === "string" && (value === "useful" || value === "irrelevant"))
    .slice(0, 50)
    .map(([key, value]) => [key.slice(0, 80), value]);
  return entries.length > 0 ? Object.fromEntries(entries) : undefined;
}

function normalizeSourceControls(controls) {
  if (!controls || typeof controls !== "object") return { excludedSourceKeys: [], allowedAuthors: [], allowedSourceKeys: [], selectedSources: [] };
  const excludedSourceKeys = Array.isArray(controls.excludedSourceKeys)
    ? controls.excludedSourceKeys
        .filter((key) => typeof key === "string" && key.trim())
        .map((key) => key.trim().slice(0, 360))
        .slice(0, 100)
    : [];
  const allowedSourceKeys = Array.isArray(controls.allowedSourceKeys)
    ? controls.allowedSourceKeys
        .filter((key) => typeof key === "string" && key.trim())
        .map((key) => key.trim().slice(0, 360))
        .slice(0, 50)
    : [];
  const allowedAuthors = Array.isArray(controls.allowedAuthors)
    ? controls.allowedAuthors
        .filter((author) => typeof author === "string" && author.trim())
        .map((author) => author.trim().slice(0, 80))
        .slice(0, 20)
    : [];
  const allowedSourceKeySet = new Set(allowedSourceKeys);
  const selectedSources = Array.isArray(controls.selectedSources)
    ? controls.selectedSources
        .map(normalizeSelectedSource)
        .filter(Boolean)
        .filter((source) => allowedSourceKeySet.size === 0 || sourceIdentityKeysForControl(source).some((key) => allowedSourceKeySet.has(key)))
        .slice(0, 12)
    : [];
  return {
    excludedSourceKeys: [...new Set(excludedSourceKeys)],
    allowedAuthors: [...new Set(allowedAuthors)],
    allowedSourceKeys: [...new Set(allowedSourceKeys)],
    selectedSources,
  };
}

function sourceControlsHasAnyValue(controls) {
  return (controls?.excludedSourceKeys || []).length > 0 || (controls?.allowedAuthors || []).length > 0 || (controls?.allowedSourceKeys || []).length > 0 || (controls?.selectedSources || []).length > 0;
}

function normalizeUserSourceControls(controls) {
  if (!controls || typeof controls !== "object") return { enabledIds: [], mode: "blend" };
  const enabledIds = Array.isArray(controls.enabledIds)
    ? controls.enabledIds
        .filter((id) => typeof id === "string" && id.trim())
        .map((id) => String(id).trim().replace(/[^a-zA-Z0-9_.-]/g, "-").slice(0, 100))
        .filter(Boolean)
        .slice(0, 20)
    : [];
  const mode = controls.mode === "only" ? "only" : "blend";
  return { enabledIds: [...new Set(enabledIds)], mode };
}

function normalizeSelectedSource(source) {
  if (!source || typeof source !== "object") return null;
  const safeText = (value, max = 260) => String(value || "").trim().slice(0, max);
  const normalized = {
    kind: safeText(source.kind, 40),
    author: safeText(source.author, 80),
    date: safeText(source.date, 32),
    title: safeText(source.title || source.label, 180),
    excerpt: safeText(source.excerpt || source.quote || source.text || source.label, 700),
    sourceUrl: safeText(source.sourceUrl, 360),
    sourcePath: safeText(source.sourcePath, 360),
    sourceKey: safeText(source.sourceKey, 360),
  };
  if (!normalized.title && !normalized.excerpt) return null;
  return normalized;
}

function sourceIdentityKeysForControl(source) {
  return [
    source?.sourceKey,
    source?.sourcePath,
    source?.sourceUrl,
    [source?.author, source?.date, source?.title].filter(Boolean).join("|"),
  ].filter((key) => typeof key === "string" && key.trim()).map((key) => key.trim());
}

async function selectedSourceContextText(context, selectedSources = []) {
  const rows = Array.isArray(selectedSources) ? selectedSources.map(normalizeSelectedSource).filter(Boolean).slice(0, 8) : [];
  if (!rows.length) return "";
  const blocks = [];
  for (const source of rows) {
    const article = await findSourceArticleInMemoryDb(context, source);
    if (article?.content) {
      blocks.push(article.content.trim());
      continue;
    }
    const title = source.title || "未命名来源";
    const author = source.author || "跨境电商长期主义";
    const date = source.date || "1970-01-01";
    const lines = [`${author} ${date} ${title}: # ${title}`];
    lines.push(`作者：${author}`);
    lines.push(`发布时间：${date}`);
    if (source.sourceUrl) lines.push(`原文链接：${source.sourceUrl}`);
    if (source.sourcePath) lines.push(`来源文件：${source.sourcePath}`);
    lines.push("来源状态：候选/待确认，必须先核对，不能直接当成已采纳证据。");
    if (source.excerpt) lines.push(source.excerpt);
    blocks.push(lines.join("\n"));
  }
  return blocks.join("\n\n");
}

function sourceControlsFromHistory(history) {
  const safeHistory = Array.isArray(history) ? history : [];
  for (let index = safeHistory.length - 1; index >= 0; index -= 1) {
    const entry = safeHistory[index];
    if (entry?.role !== "assistant") continue;
    const controls = normalizeSourceControls(entry.sourceControls);
    if (sourceControlsHasAnyValue(controls)) return controls;
    const scope = normalizeSourceScope(entry.sourceScope);
    if (scope?.active && scope.allowedAuthors.length > 0) {
      return { excludedSourceKeys: [], allowedAuthors: scope.allowedAuthors, allowedSourceKeys: scope.allowedSourceKeys || [] };
    }
  }
  return normalizeSourceControls();
}

function normalizeSourceScope(scope) {
  if (!scope || typeof scope !== "object") return undefined;
  const allowedAuthors = Array.isArray(scope.allowedAuthors)
    ? scope.allowedAuthors
        .filter((author) => typeof author === "string" && author.trim())
        .map((author) => author.trim().slice(0, 80))
        .slice(0, 20)
    : [];
  const allowedSourceKeys = Array.isArray(scope.allowedSourceKeys)
    ? scope.allowedSourceKeys
        .filter((key) => typeof key === "string" && key.trim())
        .map((key) => key.trim().slice(0, 360))
        .slice(0, 50)
    : [];
  return {
    active: Boolean(scope.active || allowedAuthors.length > 0 || allowedSourceKeys.length > 0),
    allowedAuthors,
    allowedSourceKeys,
    allowedSourceCount: Number.isFinite(Number(scope.allowedSourceCount)) ? Math.max(0, Math.floor(Number(scope.allowedSourceCount))) : allowedSourceKeys.length,
    totalRetrieved: Number.isFinite(Number(scope.totalRetrieved)) ? Number(scope.totalRetrieved) : 0,
    totalAfterScope: Number.isFinite(Number(scope.totalAfterScope)) ? Number(scope.totalAfterScope) : 0,
    summary: String(scope.summary || "").slice(0, 240),
    caution: String(scope.caution || "").slice(0, 240),
  };
}

function normalizeLearningCard(card) {
  if (!card || typeof card !== "object") return undefined;
  const safeText = (value, max = 240) => String(value || "").slice(0, max);
  const safeList = (items, maxItems = 6, maxText = 220) =>
    Array.isArray(items) ? items.map((item) => safeText(item, maxText)).filter(Boolean).slice(0, maxItems) : [];
  const evidence = Array.isArray(card.evidence)
    ? card.evidence
        .map((item, index) => ({
          sourceIndex: Number.isInteger(item?.sourceIndex) ? item.sourceIndex : index,
          title: safeText(item?.title, 180),
          author: safeText(item?.author, 80),
          date: safeText(item?.date, 32),
        }))
        .filter((item) => item.title)
        .slice(0, 5)
    : [];

  return {
    intent: {
      type: safeText(card.intent?.type || "general", 40),
      label: safeText(card.intent?.label || "学习问题", 80),
      description: safeText(card.intent?.description || "", 180),
    },
    takeaway: safeText(card.takeaway, 260),
    conclusions: safeList(card.conclusions, 4),
    nextActions: safeList(card.nextActions, 6),
    missingInputs: safeList(card.missingInputs, 6),
    followUps: safeList(card.followUps, 5),
    evidence,
    studyChecks: Array.isArray(card.studyChecks)
      ? card.studyChecks
          .map((item, index) => ({
            id: safeText(item?.id || `check:${index}`, 60),
            kind: safeText(item?.kind, 40),
            question: safeText(item?.question, 180),
            expectedAnswer: safeText(item?.expectedAnswer, 320),
            prompt: safeText(item?.prompt, 360),
            sourceIndex: Number.isInteger(item?.sourceIndex) ? item.sourceIndex : undefined,
            boundary: safeText(item?.boundary, 220),
          }))
          .filter((item) => item.question && item.expectedAnswer)
          .slice(0, 4)
      : [],
  };
}

async function saveDossier(context, body) {
  const createdAt = new Date().toISOString();
  const id = safeDossierId(body?.id || dossierIdFromBody(body, createdAt));
  const dossier = normalizeStoredDossier(buildLearningDossier({
    id,
    createdAt,
    question: body?.question,
    message: body?.message,
    sourceControls: sourceControlsForDossier(body),
  }));
  const verifiedSourceKeys = await verifiedSourceKeysForDossier(context, dossier);
  const validation = validateLearningDossierForSave(dossier, {
    message: body?.message,
    requireSourceAuthenticity: true,
    verifiedSourceKeys,
  });
  if (!validation.ok) {
    throw userFacingError(validation.message, 400);
  }
  return writeDossier(context.namespace, dossier);
}

async function verifiedSourceKeysForDossier(context, dossier) {
  const keys = new Set();
  const acceptedEvidence = Array.isArray(dossier?.acceptedEvidence) ? dossier.acceptedEvidence : [];
  for (const evidence of acceptedEvidence) {
    const source = sourceRequestFromEvidence(evidence);
    const article = await findSourceArticleInMemoryDb(context, source);
    const content = String(article?.content || "");
    const quote = String(evidence?.quote || evidence?.text || "");
    if (!content || !quote || !sourceArticleContainsQuote(content, quote)) continue;
    sourceIdentityKeysForControl({
      ...source,
      sourceKey: evidence.sourceKey,
      sourcePath: source.sourcePath || article?.metadata?.source_path || article?.metadata?.sourcePath || "",
      sourceUrl: source.sourceUrl || article?.metadata?.source_url || article?.metadata?.sourceUrl || "",
      title: source.title || article?.title || article?.metadata?.title || "",
      author: source.author || article?.metadata?.author || "",
      date: source.date || article?.metadata?.date || "",
    }).forEach((key) => keys.add(key));
  }
  return [...keys];
}

function sourceRequestFromEvidence(evidence = {}) {
  return {
    author: safeSourceText(evidence.author, 80),
    date: safeSourceText(evidence.date, 24),
    title: safeSourceText(evidence.title, 220),
    sourceUrl: safeSourceText(evidence.sourceUrl, 700),
    sourcePath: safeSourceText(evidence.sourcePath, 700),
    excerpt: safeSourceText(evidence.quote || evidence.text, 1800),
    sourceKey: safeSourceText(evidence.sourceKey, 700),
  };
}

function sourceArticleContainsQuote(content, quote) {
  const sourceText = normalizeEvidenceSourceText(content);
  const quoteText = normalizeEvidenceSourceText(quote);
  return !!sourceText && !!quoteText && sourceText.includes(quoteText);
}

function normalizeEvidenceSourceText(value) {
  return String(value || "")
    .replace(/[【】\[\]（）()《》「」“”"'`]/g, "")
    .replace(/\s+/g, "")
    .trim();
}

function sourceControlsForDossier(body) {
  const controls = normalizeSourceControls(body?.sourceControls);
  const messageScope = normalizeSourceScope(body?.message?.sourceScope);
  if (messageScope?.active && messageScope.allowedAuthors.length > 0) {
    return {
      ...controls,
      allowedAuthors: messageScope.allowedAuthors,
    };
  }
  return controls;
}

async function listDossiers(namespace) {
  const dossiers = await readAllDossiers(namespace);
  return dossiers.map(dossierSummary).sort((a, b) => String(b.createdAt).localeCompare(String(a.createdAt))).slice(0, 100);
}

async function readAllDossiers(namespace) {
  const dir = await ensureDossierDir(namespace);
  const files = await readdir(dir);
  const dossiers = [];
  for (const file of files) {
    if (!file.endsWith(".json")) continue;
    try {
      const dossier = normalizeStoredDossier(JSON.parse(await readFile(path.join(dir, file), "utf8")));
      if (dossier.id) dossiers.push(dossier);
    } catch {
      // Ignore malformed local archive files so one bad file does not break the entry.
    }
  }
  return dossiers.sort((a, b) => String(b.createdAt).localeCompare(String(a.createdAt))).slice(0, 100);
}

async function readDossier(namespace, id) {
  const safeId = safeDossierId(id);
  return normalizeStoredDossier(JSON.parse(await readFile(dossierPath(namespace, safeId), "utf8")));
}

async function updateDossierReview(namespace, id, body) {
  const safeId = safeDossierId(id);
  return withDossierWriteLock(namespace, safeId, async () => {
    const dossier = await readDossier(namespace, safeId);
    const updated = updateDossierReviewState(dossier, {
      checked: body?.checked,
      updatedAt: new Date().toISOString(),
    });
    return writeDossier(namespace, updated);
  });
}

async function updateDossierSelfTest(namespace, id, body) {
  const safeId = safeDossierId(id);
  return withDossierWriteLock(namespace, safeId, async () => {
    const dossier = await readDossier(namespace, safeId);
    const updated = updateDossierSelfTestState(dossier, {
      mastered: body?.mastered,
      updatedAt: new Date().toISOString(),
    });
    return writeDossier(namespace, updated);
  });
}

async function updateDossierEvidenceDecision(namespace, id, body) {
  const safeId = safeDossierId(id);
  return withDossierWriteLock(namespace, safeId, async () => {
    const dossier = await readDossier(namespace, safeId);
    const updated = updateDossierEvidenceDecisionState(dossier, {
      sourceIndex: Number(body?.sourceIndex),
      decision: body?.decision,
    });
    return writeDossier(namespace, updated);
  });
}

async function updateDossierBusinessVerification(namespace, id, body) {
  const safeId = safeDossierId(id);
  return withDossierWriteLock(namespace, safeId, async () => {
    const dossier = await readDossier(namespace, safeId);
    const updated = updateDossierBusinessVerificationState(dossier, {
      text: body?.text,
      createdAt: new Date().toISOString(),
    });
    return writeDossier(namespace, updated);
  });
}

async function updateDossierExperimentResult(namespace, id, body) {
  const safeId = safeDossierId(id);
  return withDossierWriteLock(namespace, safeId, async () => {
    const dossier = await readDossier(namespace, safeId);
    const updated = updateDossierExperimentResultState(dossier, {
      text: body?.text,
      createdAt: new Date().toISOString(),
    });
    return writeDossier(namespace, updated);
  });
}

async function writeDossier(namespace, dossier) {
  const normalized = normalizeStoredDossier(dossier);
  await ensureDossierDir(namespace);
  await writeFile(dossierPath(namespace, normalized.id), `${JSON.stringify(normalized, null, 2)}\n`, "utf8");
  return normalized;
}

async function syncDossierToOpenHumanMemory(context, dossier) {
  const normalized = normalizeStoredDossier(dossier);
  const memoryDoc = buildOpenHumanMemoryDocument(normalized, { sourceNamespace: context.namespace });
  const baseRecord = {
    namespace: memoryDoc.namespace,
    key: memoryDoc.key,
    documentTitle: memoryDoc.title,
    syncedAt: new Date().toISOString(),
  };
  try {
    if (memoryDoc.namespace === context.namespace) {
      throw new Error("学习档案不能写入原作者资料库。");
    }
    const result = await rpcCall(context.rpcUrl, context.token, "openhuman.memory_doc_put", memoryDoc);
    const documentId = openHumanDocumentIdFromResult(result);
    const indexState = await getMemoryDocumentIndexState(memoryDoc.namespace, documentId);
    const status = indexState.indexStatus === "indexed" ? "synced" : "pending";
    return writeDossier(context.namespace, {
      ...normalized,
      openhumanMemory: {
        ...baseRecord,
        status,
        documentId,
        ...indexState,
        error: "",
      },
    });
  } catch (error) {
    return writeDossier(context.namespace, {
      ...normalized,
      openhumanMemory: {
        ...baseRecord,
        status: "failed",
        error: friendlyError(error),
      },
    });
  }
}

function assertDossierCanAdvance(dossier) {
  const normalized = normalizeStoredDossier(dossier);
  if (normalized.acceptedEvidence.length === 0) {
    throw userFacingError("这个学习档案还没有已采纳的作者原文证据，请先确认来源是否有用。", 400);
  }
}

async function skippedMemoryAfterEvidenceRemoval(context, previousDossier) {
  await deleteOpenHumanMemoryForDossier(context, previousDossier);
  return {
    status: "skipped",
    error: "已采纳原文证据为空，暂不沉淀到 OpenHuman 本地记忆。",
  };
}

async function deleteOpenHumanMemoryForDossier(context, dossier) {
  const memory = dossier?.openhumanMemory || {};
  if (!memory.namespace || !(memory.documentId || memory.key)) return;
  try {
    await rpcCall(context.rpcUrl, context.token, "openhuman.memory_delete_document", {
      namespace: memory.namespace,
      document_id: memory.documentId || memory.key,
    });
  } catch (error) {
    if (process.env.AMAZON_QA_DEBUG) {
      console.warn(`Learning memory delete skipped: ${friendlyError(error)}`);
    }
  }
}

function openHumanDocumentIdFromResult(result) {
  return result?.result?.document_id ||
    result?.result?.documentId ||
    result?.document_id ||
    result?.documentId ||
    result?.id ||
    "";
}

async function getMemoryDocumentIndexState(namespace, documentId) {
  if (!existsSync(DB_PATH) || !documentId) {
    return { indexStatus: "unknown", totalChunks: 0, indexedChunks: 0 };
  }
  const ns = quoteSql(namespace);
  const doc = quoteSql(documentId);
  const sql = [
    `select`,
    `(select count(*) from vector_chunks where namespace='${ns}' and document_id='${doc}') || '|' ||`,
    `(select count(*) from vector_chunks where namespace='${ns}' and document_id='${doc}' and embedding is not null and length(embedding)>0);`,
  ].join(" ");
  try {
    const { stdout } = await execFileAsync("sqlite3", [DB_PATH, sql], { timeout: 3000 });
    const [totalChunks, indexedChunks] = stdout
      .trim()
      .split("|")
      .map((value) => Number(value || 0));
    const indexStatus = totalChunks > 0 && indexedChunks >= totalChunks ? "indexed" : "missing_embeddings";
    return { indexStatus, totalChunks, indexedChunks };
  } catch {
    return { indexStatus: "unknown", totalChunks: 0, indexedChunks: 0 };
  }
}

async function withDossierWriteLock(namespace, id, operation) {
  const key = `${safeNamespace(namespace)}:${safeDossierId(id)}`;
  const previous = dossierWriteQueues.get(key) || Promise.resolve();
  const run = previous.catch(() => {}).then(operation);
  const cleanup = run.finally(() => {
    if (dossierWriteQueues.get(key) === cleanup) dossierWriteQueues.delete(key);
  });
  dossierWriteQueues.set(key, cleanup);
  return run;
}

async function deleteDossier(context, namespace, id) {
  const safeId = safeDossierId(id);
  const dossier = await readDossier(namespace, safeId).catch(() => null);
  await deleteOpenHumanMemoryForDossier(context, dossier);
  await unlink(dossierPath(namespace, safeId));
}

function userFacingError(message, statusCode = 400) {
  const error = new Error(message);
  error.statusCode = statusCode;
  return error;
}

async function ensureDossierDir(namespace) {
  const dir = path.join(DOSSIER_ROOT, safeNamespace(namespace));
  await mkdir(dir, { recursive: true });
  return dir;
}

function dossierPath(namespace, id) {
  return path.join(DOSSIER_ROOT, safeNamespace(namespace), `${safeDossierId(id)}.json`);
}

function dossierSummary(dossier) {
  const diagnosisTracks = Array.isArray(dossier.diagnosisPanel?.tracks) ? dossier.diagnosisPanel.tracks : [];
  const diagnosisChecks = diagnosisTracks.reduce((count, track) => count + (Array.isArray(track.checks) ? track.checks.length : 0), 0);
  const diagnosisChecked = Object.values(dossier.diagnosisPanel?.checked || {}).filter((value) => value === true).length;
  const workbench = buildDossierWorkbench(dossier);
  const reviewQueue = workbench.reviewQueue;
  const selfTest = workbench.selfTest;
  const businessVerification = workbench.businessVerification;
  return {
    id: dossier.id,
    createdAt: dossier.createdAt,
    title: dossier.title,
    question: dossier.question,
    takeaway: dossier.takeaway,
    counts: {
      acceptedEvidence: dossier.acceptedEvidence.length,
      rejectedEvidence: dossier.rejectedEvidence.length,
      excludedSources: dossier.excludedSources.length,
      sources: dossier.sources.length,
      productFactGroups: Array.isArray(dossier.productInputSummary?.facts) ? dossier.productInputSummary.facts.length : 0,
      diagnosisTracks: diagnosisTracks.length,
      diagnosisChecks,
      diagnosisChecked,
      reviewTotal: reviewQueue?.progress?.total || 0,
      reviewCompleted: reviewQueue?.progress?.completed || 0,
      selfTestTotal: selfTest?.progress?.total || 0,
      selfTestMastered: selfTest?.progress?.mastered || 0,
      businessVerificationRecords: dossier.businessVerificationRecords.length,
      businessVerificationReady: dossier.businessVerificationRecords.filter((record) => record.status === "ready").length,
      businessVerificationDimensionsReady: businessVerification?.coverage?.ready || 0,
      businessVerificationDimensionsTotal: businessVerification?.totalRecords ? businessVerification?.coverage?.total || 0 : 0,
      experimentResults: dossier.experimentResultRecords.length,
      experimentResultsPositive: dossier.experimentResultRecords.filter((record) => record.outcome === "positive").length,
    },
    openhumanMemory: dossier.openhumanMemory,
    researchMission: workbench ? buildDossierOverview([dossier]).researchMissions?.[0] || null : null,
    nextActions: dossier.nextActions.slice(0, 3),
  };
}

function dossierResponsePayload(dossier) {
  const normalized = normalizeStoredDossier(dossier);
  return {
    dossier: normalized,
    summary: dossierSummary(normalized),
    workbench: buildDossierWorkbench(normalized),
    persistence: dossierPersistenceState(normalized),
  };
}

function dossierResponseStatus(payload) {
  return ["failed", "pending"].includes(payload?.persistence?.memory?.status) ? 207 : 200;
}

function dossierPersistenceState(dossier) {
  const memory = dossier.openhumanMemory || {};
  const memoryStatus = memory.status === "synced"
    ? "saved"
    : memory.status === "failed"
      ? "failed"
      : memory.status === "pending"
        ? "pending"
        : "skipped";
  const memoryMessage = memoryStatus === "saved"
    ? "已沉淀到 OpenHuman 本地记忆"
    : memoryStatus === "failed"
      ? "学习档案已保存，但沉淀到 OpenHuman 本地记忆失败"
      : memoryStatus === "pending"
        ? memory.indexStatus === "missing_embeddings"
          ? "已保存到 OpenHuman 本地记忆，语义索引未完成"
          : "正在沉淀到 OpenHuman 本地记忆"
        : "学习档案已保存，尚未沉淀到 OpenHuman 本地记忆";

  return {
    archive: {
      status: "saved",
      id: dossier.id,
      savedAt: dossier.createdAt,
    },
    memory: {
      status: memoryStatus,
      namespace: memory.namespace || "",
      key: memory.key || "",
      documentId: memory.documentId || "",
      documentTitle: memory.documentTitle || "",
      syncedAt: memory.syncedAt || "",
      indexStatus: memory.indexStatus || "",
      totalChunks: memory.totalChunks || 0,
      indexedChunks: memory.indexedChunks || 0,
      message: memoryMessage,
      error: memory.error || "",
    },
  };
}

function dossierIdFromBody(body, createdAt) {
  const seed = JSON.stringify({
    createdAt: body?.message?.createdAt || body?.createdAt || createdAt,
    question: body?.question || "",
    content: String(body?.message?.content || "").slice(0, 240),
  });
  const date = String(body?.message?.createdAt || createdAt).slice(0, 10).replace(/\D/g, "") || "archive";
  const hash = createHash("sha1").update(seed).digest("hex").slice(0, 12);
  return `amazon-${date}-${hash}`;
}

function safeNamespace(namespace) {
  return String(namespace || DEFAULT_NAMESPACE).replace(/[^a-zA-Z0-9_.-]/g, "-").slice(0, 80) || DEFAULT_NAMESPACE;
}

function safeDossierId(id) {
  const safe = String(id || "").replace(/[^a-zA-Z0-9_.-]/g, "-").slice(0, 100);
  if (!safe) throw new Error("无效的学习档案编号。");
  return safe;
}

function removeCurrentQuestionFromSeedHistory(history, question) {
  const normalizedQuestion = String(question || "").trim();
  const seeded = [...history];
  while (
    seeded.length > 0 &&
    seeded[seeded.length - 1].role === "user" &&
    seeded[seeded.length - 1].content.trim() === normalizedQuestion
  ) {
    seeded.pop();
  }
  return seeded;
}

function pruneSessions() {
  const maxAgeMs = 12 * 60 * 60 * 1000;
  const now = Date.now();
  for (const [id, session] of sessions.entries()) {
    if (now - session.updatedAt > maxAgeMs) sessions.delete(id);
  }
}

async function ensureCoreRunning({ coreBaseUrl, corePort, token }) {
  if (await isHealthy(coreBaseUrl)) return;
  if (!existsSync(CORE_BIN)) {
    throw new Error(`OpenHuman core is not built: ${CORE_BIN}`);
  }

  coreProcess = spawn(CORE_BIN, ["run", "--host", DEFAULT_HOST, "--port", String(corePort), "--jsonrpc-only"], {
    cwd: path.dirname(CORE_BIN),
    env: {
      ...process.env,
      HOME: process.env.HOME || "/Users/yangyingjia",
      USER: process.env.USER || "yangyingjia",
      PATH: `/Users/yangyingjia/.cargo/bin:/opt/homebrew/bin:${process.env.PATH || ""}`,
      OPENHUMAN_WORKSPACE: WORKSPACE,
      OPENHUMAN_CORE_TOKEN: token,
      RUST_LOG: process.env.RUST_LOG || "warn",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });

  coreProcess.stdout.on("data", (chunk) => {
    if (process.env.AMAZON_QA_DEBUG) process.stdout.write(chunk);
  });
  coreProcess.stderr.on("data", (chunk) => {
    if (process.env.AMAZON_QA_DEBUG) process.stderr.write(chunk);
  });

  const started = await waitForHealth(coreBaseUrl, 25);
  if (!started) {
    throw new Error("OpenHuman core did not become healthy in time");
  }
}

async function waitForHealth(baseUrl, attempts) {
  for (let index = 0; index < attempts; index += 1) {
    if (await isHealthy(baseUrl)) return true;
    await sleep(400);
  }
  return false;
}

async function isHealthy(baseUrl) {
  try {
    const response = await fetch(`${baseUrl}/health`, { signal: AbortSignal.timeout(700) });
    if (!response.ok) return false;
    const payload = await response.json();
    return payload?.ok === true;
  } catch {
    return false;
  }
}

async function startSourceTreeDrain(context, body = {}) {
  const rows = await readManifestRows();
  const sourceTree = await getSourceTreeHealth(rows);
  const current = await getSourceTreeDrainStatus(sourceTree);
  if (!current.canStart) return current;
  if (!existsSync(SOURCE_TREE_DRAIN_RUNNER)) {
    throw new Error("来源树深加工运行器不存在。");
  }
  const preflight = await checkSourceTreeDrainPreflight();
  if (!preflight.ok) {
    await writeSourceTreeDrainState({
      state: "preflight_failed",
      error: preflight.message,
      preflight,
      processedJobs: 0,
      maxJobs: null,
      batchSize: null,
      queuedJobs: sourceTree.queuedJobs || 0,
      readyJobs: sourceTree.readyJobs || 0,
      runningJobs: sourceTree.runningJobs || 0,
      failedJobs: sourceTree.failedJobs || 0,
      doneJobs: sourceTree.doneJobs || 0,
      activeBatch: null,
      lastBatch: null,
      startedAt: new Date().toISOString(),
      finishedAt: new Date().toISOString(),
    });
    return getSourceTreeDrainStatus(sourceTree);
  }

  const maxJobs = clampNumber(body?.maxJobs, 250, 1, 250);
  const batchSize = clampNumber(body?.batchSize, 25, 1, 100);
  const sleepMs = clampNumber(body?.sleepMs, 750, 0, 60000);
  await unlink(SOURCE_TREE_DRAIN_STOP_PATH).catch(() => {});
  await writeSourceTreeDrainState({
    state: "starting",
    pid: null,
    processedJobs: 0,
    maxJobs,
    batchSize,
    queuedJobs: sourceTree.queuedJobs || 0,
    readyJobs: sourceTree.readyJobs || 0,
    runningJobs: sourceTree.runningJobs || 0,
    failedJobs: sourceTree.failedJobs || 0,
    doneJobs: sourceTree.doneJobs || 0,
    startedAt: new Date().toISOString(),
  });

  sourceTreeDrainProcess = spawn(process.execPath, [
    SOURCE_TREE_DRAIN_RUNNER,
    "--max-jobs",
    String(maxJobs),
    "--batch-size",
    String(batchSize),
    "--sleep-ms",
    String(sleepMs),
  ], {
    cwd: ROOT,
    detached: true,
    env: {
      ...process.env,
      PATH: `/Users/yangyingjia/.cargo/bin:/opt/homebrew/bin:${process.env.PATH || ""}`,
      OPENHUMAN_WORKSPACE: WORKSPACE,
      RUST_LOG: process.env.RUST_LOG || "warn",
    },
    stdio: "ignore",
  });
  sourceTreeDrainProcess.unref();
  await sleep(250);
  return getSourceTreeDrainStatus(sourceTree);
}

async function checkSourceTreeDrainPreflight() {
  const configText = await readFile(path.join(WORKSPACE, "config.toml"), "utf8").catch(() => "");
  const endpoint = firstNonEmpty(
    tomlSectionValue(configText, "memory_tree", "embedding_endpoint"),
    tomlSectionValue(configText, "memory_tree", "llm_summariser_endpoint"),
    tomlSectionValue(configText, "local_ai", "base_url"),
    "http://127.0.0.1:11434",
  );
  const requiredModels = uniqueNonEmpty([
    tomlSectionValue(configText, "memory", "embedding_model"),
    tomlSectionValue(configText, "local_ai", "embedding_model_id"),
    tomlSectionValue(configText, "memory_tree", "embedding_model"),
    tomlSectionValue(configText, "memory_tree", "llm_summariser_model"),
    tomlSectionValue(configText, "memory_tree", "llm_extractor_model"),
  ]);
  try {
    const response = await fetch(`${endpoint.replace(/\/+$/, "")}/api/tags`, {
      signal: AbortSignal.timeout(2500),
    });
    if (!response.ok) {
      return buildSourceTreeDrainPreflight({
        ok: false,
        endpoint,
        requiredModels,
        error: `HTTP ${response.status}`,
      });
    }
    const payload = await response.json();
    const availableModels = Array.isArray(payload?.models)
      ? payload.models.flatMap((model) => [model?.name, model?.model]).filter(Boolean)
      : [];
    return buildSourceTreeDrainPreflight({ ok: true, endpoint, requiredModels, availableModels });
  } catch (error) {
    return buildSourceTreeDrainPreflight({
      ok: false,
      endpoint,
      requiredModels,
      error: friendlyError(error),
    });
  }
}

async function stopSourceTreeDrain(context) {
  const rows = await readManifestRows();
  const sourceTree = await getSourceTreeHealth(rows);
  const current = await getSourceTreeDrainStatus(sourceTree);
  if (!current.running) return current;
  await mkdir(RUN_DIR, { recursive: true });
  await writeFile(SOURCE_TREE_DRAIN_STOP_PATH, new Date().toISOString(), "utf8");
  await writeSourceTreeDrainState({
    state: "stopping",
    stopRequested: true,
    processedJobs: current.processedJobs || 0,
    queuedJobs: sourceTree.queuedJobs || current.queuedJobs || 0,
    readyJobs: sourceTree.readyJobs || current.readyJobs || 0,
    runningJobs: sourceTree.runningJobs || current.runningJobs || 0,
    failedJobs: sourceTree.failedJobs || current.failedJobs || 0,
    doneJobs: sourceTree.doneJobs || current.doneJobs || 0,
    startedAt: current.startedAt || new Date().toISOString(),
  });
  return getSourceTreeDrainStatus(sourceTree);
}

async function getSourceTreeDrainStatus(sourceTree = {}) {
  let run = await readJsonFile(SOURCE_TREE_DRAIN_STATE_PATH);
  const stopRequested = existsSync(SOURCE_TREE_DRAIN_STOP_PATH);
  const processAlive = run?.pid ? isProcessAlive(Number(run.pid)) : false;
  if (
    run?.pid &&
    (run.state === "running" || run.state === "starting") &&
    !processAlive
  ) {
    run = { ...run, state: "stale" };
  } else if (run?.pid) {
    run = { ...run, processAlive };
  }
  if (stopRequested && run && (run.state === "running" || run.state === "starting")) {
    run = { ...run, state: "stopping", stopRequested: true };
  }
  return summarizeSourceTreeDrain({ sourceTree, run });
}

async function writeSourceTreeDrainState(state) {
  await mkdir(RUN_DIR, { recursive: true });
  const previous = await readJsonFile(SOURCE_TREE_DRAIN_STATE_PATH);
  const tmpPath = `${SOURCE_TREE_DRAIN_STATE_PATH}.${process.pid}.tmp`;
  await writeFile(
    tmpPath,
    JSON.stringify({
      ...(previous || {}),
      ...state,
      updatedAt: new Date().toISOString(),
    }, null, 2),
    "utf8",
  );
  await rename(tmpPath, SOURCE_TREE_DRAIN_STATE_PATH);
}

async function readJsonFile(file) {
  try {
    return JSON.parse(await readFile(file, "utf8"));
  } catch {
    return null;
  }
}

function isProcessAlive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function clampNumber(value, fallback, min, max) {
  const number = Number(value);
  if (!Number.isFinite(number)) return fallback;
  return Math.min(max, Math.max(min, Math.floor(number)));
}

function firstNonEmpty(...values) {
  for (const value of values) {
    const text = String(value || "").trim();
    if (text) return text;
  }
  return "";
}

function uniqueNonEmpty(values = []) {
  return [...new Set(
    (Array.isArray(values) ? values : [])
      .map((value) => String(value || "").trim())
      .filter(Boolean),
  )];
}

function tomlSectionValue(text = "", sectionName = "", keyName = "") {
  const targetSection = String(sectionName || "").trim();
  const targetKey = String(keyName || "").trim();
  if (!targetSection || !targetKey) return "";
  let activeSection = "";
  for (const rawLine of String(text || "").split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    const sectionMatch = line.match(/^\[([^\]]+)\]$/);
    if (sectionMatch) {
      activeSection = sectionMatch[1].trim();
      continue;
    }
    if (activeSection !== targetSection) continue;
    const valueMatch = line.match(/^([A-Za-z0-9_.-]+)\s*=\s*(.*)$/);
    if (!valueMatch || valueMatch[1] !== targetKey) continue;
    return unquoteTomlValue(valueMatch[2]);
  }
  return "";
}

function unquoteTomlValue(value = "") {
  const text = String(value || "").trim();
  if ((text.startsWith("\"") && text.endsWith("\"")) || (text.startsWith("'") && text.endsWith("'"))) {
    return text.slice(1, -1).trim();
  }
  return text.split("#")[0].trim();
}

async function rpcCall(rpcUrl, token, method, params) {
  const response = await fetch(rpcUrl, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${token}`,
    },
    body: JSON.stringify({ jsonrpc: "2.0", id: Date.now(), method, params }),
  });
  const payload = await response.json();
  const envelopeError = payload.error ?? payload.result?.error;
  if (!response.ok || envelopeError) {
    throw new Error(`${method} failed: ${JSON.stringify(envelopeError ?? payload)}`);
  }
  return payload.result;
}

async function queryKnowledge(context, query, options = {}) {
  const namespace = sourceNamespaceFor(context.namespace);
  const base = await queryNamespace(context, namespace, query, 12, { fallback: true });
  const authors = Array.isArray(options.authorDiversityAuthors)
    ? options.authorDiversityAuthors.filter((author) => typeof author === "string" && author.trim()).slice(0, AMAZON_AUTHORS.length)
    : [];
  if (authors.length === 0) return base;

  const authorResults = await Promise.all(
    authors.map(async (author) => {
      const authorQuery = `${query}\n作者：${author}\n请优先返回 ${author} 的相关原文。`;
      try {
        return await queryNamespace(context, namespace, authorQuery, 5, { fallback: true });
      } catch (error) {
        if (process.env.AMAZON_QA_DEBUG) {
          console.warn(`Author diversity query skipped for ${author}: ${friendlyError(error)}`);
        }
        return "";
      }
    }),
  );
  const authorFallbackContext = await queryAuthorDiversityFromDb(context, query, authors);
  return mergeKnowledgeContexts([base, ...authorResults, authorFallbackContext]);
}

async function queryLearningMemory(context, query) {
  const namespace = workflowNamespaceFor(context.namespace);
  try {
    return filterReusableLearningMemoryContext(await queryNamespace(context, namespace, query, 4, { fallback: false }));
  } catch (error) {
    if (process.env.AMAZON_QA_DEBUG) {
      console.warn(`Learning memory query skipped: ${friendlyError(error)}`);
    }
    return { data: { context: { chunks: [] } } };
  }
}

function filterReusableLearningMemoryContext(result) {
  const filter = (chunks) => chunks.filter(isReusableLearningMemoryChunk);
  if (Array.isArray(result?.data?.context?.chunks)) {
    return { ...result, data: { ...(result.data || {}), context: { ...(result.data.context || {}), chunks: filter(result.data.context.chunks) } } };
  }
  if (Array.isArray(result?.context?.chunks)) {
    return { ...result, context: { ...(result.context || {}), chunks: filter(result.context.chunks) } };
  }
  if (Array.isArray(result?.result?.data?.context?.chunks)) {
    return {
      ...result,
      result: {
        ...(result.result || {}),
        data: {
          ...(result.result.data || {}),
          context: { ...(result.result.data.context || {}), chunks: filter(result.result.data.context.chunks) },
        },
      },
    };
  }
  if (Array.isArray(result?.result?.context?.chunks)) {
    return { ...result, result: { ...(result.result || {}), context: { ...(result.result.context || {}), chunks: filter(result.result.context.chunks) } } };
  }
  if (Array.isArray(result?.chunks)) {
    return { ...result, chunks: filter(result.chunks) };
  }
  return result;
}

function isReusableLearningMemoryChunk(chunk) {
  const content = String(chunk?.content || chunk?.text || "");
  if (!content.trim()) return false;
  if (hasAcceptedLearningEvidence(content)) return true;
  if (/产品\/ASIN|主图现状|核心关键词|竞品\/对标/.test(content)) return true;
  if (/实验复盘|实验结果|小实验|CTR\s*从|CVR\s*从|ACOS\s*从/.test(content)) return true;
  if (/\b(CTR|CVR|ACOS|CPC)\b\s*[:：]?\s*\d/.test(content)) return true;
  return false;
}

function hasAcceptedLearningEvidence(content) {
  const match = String(content || "").match(/##\s*已采纳原文证据\s*\n([\s\S]*?)(?:\n##\s|\s*$)/);
  if (!match) return false;
  const body = match[1]
    .split(/\n+/)
    .map((line) => line.replace(/^[-*\d.)、\s]+/, "").trim())
    .filter(Boolean)
    .join("\n");
  return body.length > 0 && !/^(暂无|无|没有)[。.\s]*$/.test(body);
}

async function queryUserSources(namespace, query, controls = {}) {
  const normalized = normalizeUserSourceControls(controls);
  if (normalized.enabledIds.length === 0) return { contextText: "", sources: [] };
  const fullSources = [];
  for (const id of normalized.enabledIds) {
    const source = await readUserSourceIfExists(namespace, id);
    if (source?.content) fullSources.push(source);
  }
  if (fullSources.length === 0) return { contextText: "", sources: [] };

  const terms = userSourceSearchTerms(query);
  const ranked = fullSources
    .map((source) => ({ source, score: userSourceScore(source, terms) }))
    .filter((row) => row.score > 0 || normalized.mode === "only" || fullSources.length <= 3)
    .sort((a, b) => b.score - a.score || Date.parse(b.source.updatedAt || 0) - Date.parse(a.source.updatedAt || 0))
    .slice(0, 5);
  return {
    sources: ranked.map((row) => userSourceSummary(row.source)),
    contextText: ranked.map((row) => userSourceContextBlock(row.source)).join("\n\n"),
  };
}

function userSourceSearchTerms(query) {
  const expanded = sourceTreeSearchTerms(query, 12);
  const raw = String(query || "")
    .split(/[^\p{Script=Han}A-Za-z0-9_+-]+/u)
    .map((term) => term.trim())
    .filter((term) => term.length >= 2 && term.length <= 32)
    .slice(0, 16);
  return [...new Set([...expanded, ...raw])].slice(0, 20);
}

function userSourceScore(source, terms) {
  const title = String(source.title || "").toLowerCase();
  const haystack = `${source.title}\n${source.content}`.toLowerCase();
  return terms.reduce((score, term) => {
    const safeTerm = String(term || "").toLowerCase();
    if (!safeTerm) return score;
    return score + (title.includes(safeTerm) ? 4 : 0) + (haystack.includes(safeTerm) ? 2 : 0);
  }, 0);
}

function userSourceContextBlock(source) {
  const updated = String(source.updatedAt || source.createdAt || new Date().toISOString()).slice(0, 10);
  return [
    `# ${source.title || "我的资料"}`,
    `作者：${USER_SOURCE_AUTHOR}`,
    `发布时间：${updated}`,
    `原文链接：user-source://${source.id}`,
    `来源文件：user-sources/${source.id}.json`,
    source.content,
  ].join("\n");
}

async function querySourceTreeContext(context, query, sourceControls = {}) {
  const terms = sourceTreeSearchTerms(query, 12);
  if (!existsSync(MEMORY_TREE_DB_PATH) || terms.length === 0) {
    return {
      contextText: "",
      calibration: buildSourceTreeCalibration({ query, terms, chunkRows: [], summaryRows: [], resolvedSources: [] }),
    };
  }

  try {
    const chunkRows = await querySourceTreeChunkCandidates(query, terms, sourceControls);
    const summaryRows = await querySourceTreeSummaryHints(terms);
    const resolvedSources = [];
    const contextBlocks = [];

    for (const candidate of chunkRows.slice(0, 5)) {
      const article = await findSourceArticleInMemoryDb(context, {
        sourcePath: candidate.sourceId,
        sourceUrl: candidate.sourceRef,
        author: candidate.owner,
      });
      if (!article?.content) continue;
      const metadata = article.metadata || {};
      resolvedSources.push({
        sourceId: candidate.sourceId,
        sourceRef: candidate.sourceRef,
        owner: candidate.owner,
        title: article.title || metadata.title || "",
      });
      contextBlocks.push(article.content.trim());
    }

    return {
      contextText: contextBlocks.join("\n\n"),
      calibration: buildSourceTreeCalibration({ query, terms, chunkRows, summaryRows, resolvedSources }),
    };
  } catch (error) {
    if (process.env.AMAZON_QA_DEBUG) {
      console.warn(`Source tree query skipped: ${friendlyError(error)}`);
    }
    return {
      contextText: "",
      calibration: buildSourceTreeCalibration({ query, terms, chunkRows: [], summaryRows: [], resolvedSources: [] }),
    };
  }
}

async function buildQuestionSourceSelection(context, body = {}) {
  const question = String(body.question || "").trim();
  if (question.length < 2) {
    const error = new Error("问题太短了，请先输入更具体的问题。");
    error.statusCode = 400;
    throw error;
  }
  const history = Array.isArray(body.history)
    ? removeCurrentQuestionFromSeedHistory(normalizeSessionHistory(body.history), question)
    : [];
  const sourceControls = normalizeSourceControls(body.sourceControls);
  const retrievalQuery = buildRetrievalQuery(question, history, {
    excludedSourceKeys: sourceControls.excludedSourceKeys,
  });
  const scopedRetrievalQuery = sourceControls.allowedAuthors.length > 0
    ? `${retrievalQuery}\n资料范围：${sourceControls.allowedAuthors.join("、")}`
    : retrievalQuery;
  const [sourceTreeContext, retrieval] = await Promise.all([
    querySourceTreeContext(context, scopedRetrievalQuery, sourceControls),
    queryKnowledge(context, scopedRetrievalQuery),
  ]);
  const articles = dedupeSelectionSources(
    parseOpenHumanContext(mergeKnowledgeContexts([sourceTreeContext.contextText, retrieval]))
      .map((article) => selectedSourceFromArticle(article))
      .filter(Boolean),
  ).slice(0, 6);
  const recommended = articles.slice(0, 3);
  const authors = [...new Set(recommended.map((source) => source.author).filter(Boolean))].slice(0, 4);
  const allowedSourceKeys = uniqueSourceKeysForSelection(recommended);
  const status = recommended.length > 0 ? "ready" : "needs_source";
  const intent = buildWorkflowIntent({
    question,
    retrievalQuestion: scopedRetrievalQuery,
    sources: recommended,
  });
  const intentChoices = buildQuestionIntentChoices(intent, status);
  return {
    title: "问前资料选择",
    status,
    question: question.slice(0, 220),
    summary: recommended.length > 0
      ? `建议先用 ${recommended.length} 条最相关来源回答，再根据答案后的资料选择继续扩展。`
      : "本轮没有找到足够明确的原文来源，建议先换更具体的问题或补作者资料。",
    criteria: [
      {
        id: "semantic-hit",
        label: "语义命中",
        status: recommended.length > 0 ? "ready" : "missing",
        detail: recommended.length > 0 ? "已找到可回到原文核对的候选来源。" : "没有足够明确的候选来源。",
      },
      {
        id: "source-tree",
        label: "来源树辅助",
        status: sourceTreeContext.calibration?.status || "empty",
        detail: sourceTreeContext.calibration?.summary || "来源树只帮助找路，不作为作者证据。",
      },
      {
        id: "scope-boundary",
        label: "范围可撤销",
        status: "ready",
        detail: "应用后只影响下一轮回答范围，不会改动知识库内容。",
      },
    ],
    recommended,
    alternatives: articles.slice(3, 6),
    intent,
    intentChoices,
    sourceControls: {
      excludedSourceKeys: sourceControls.excludedSourceKeys,
      allowedAuthors: authors,
      allowedSourceKeys,
      selectedSources: recommended,
    },
    sourceTreeCalibration: sourceTreeContext.calibration,
    boundary: "问前资料选择只决定下一轮先读哪些来源；候选理由是系统整理，不是新的作者原文证据，也不会写入知识库。",
  };
}

function buildQuestionIntentChoices(primaryIntent, sourceStatus) {
  const primaryType = normalizeIntentPreference(primaryIntent) || (sourceStatus === "needs_source" ? "source_search" : "method_learning");
  const types = [primaryType, "method_learning", "product_diagnosis", "experiment_review", "source_search"]
    .filter((type, index, rows) => type && rows.indexOf(type) === index)
    .slice(0, 4);
  return types.map((type) => {
    const intent = type === primaryType
      ? primaryIntent
      : workflowIntentTemplate(type, { confidence: "user_selectable" });
    return {
      type: intent.type,
      label: intent.label,
      goal: intent.goal,
      primaryAction: intent.primaryAction,
      boundary: intent.boundary,
      confidence: type === primaryType ? intent.confidence || "medium" : "user_selectable",
      recommended: type === primaryType,
    };
  });
}

function selectedSourceFromArticle(article) {
  if (!article || typeof article !== "object") return null;
  const source = normalizeSelectedSource({
    kind: "author_original",
    author: article.author,
    date: article.date,
    title: article.title,
    excerpt: article.excerpt || article.body,
    sourceUrl: article.sourceUrl,
    sourcePath: article.sourcePath,
  });
  if (!source) return null;
  source.sourceKey = source.sourcePath || source.sourceUrl || [source.author, source.date, source.title].filter(Boolean).join("|");
  source.reason = "本轮问题语义命中，适合先作为回答范围。";
  source.sourceCanUseAsEvidence = true;
  source.canUseAsEvidence = false;
  return source;
}

function dedupeSelectionSources(sources = []) {
  const seen = new Set();
  const rows = [];
  for (const source of Array.isArray(sources) ? sources : []) {
    const keys = sourceIdentityKeysForControl(source);
    const key = keys.find(Boolean) || `${source.author}|${source.title}`;
    if (!key || seen.has(key)) continue;
    keys.forEach((item) => seen.add(item));
    rows.push(source);
  }
  return rows;
}

function uniqueSourceKeysForSelection(sources = []) {
  const keys = [];
  for (const source of Array.isArray(sources) ? sources : []) {
    for (const key of sourceIdentityKeysForControl(source)) {
      if (key && !keys.includes(key)) keys.push(key);
    }
  }
  return keys.slice(0, 50);
}

async function querySourceTreeChunkCandidates(query, terms, sourceControls = {}) {
  const matchExpr = sourceTreeMatchExpr("c", terms);
  const where = [
    "c.content is not null",
    "length(c.content)>0",
    "(c.lifecycle_status is null or c.lifecycle_status!='dropped')",
    "coalesce(s.dropped,0)=0",
    `(${sourceTreeMatchWhere("c", terms)})`,
    ...sourceTreeScopeClauses(sourceControls, "c"),
  ].filter(Boolean);
  const sql = [
    "select c.source_id as sourceId, c.source_ref as sourceRef, c.owner as owner,",
    `max(${matchExpr}) as matchScore, count(*) as chunkCount, max(coalesce(s.total,0)) as treeScore`,
    "from mem_tree_chunks c",
    "left join mem_tree_score s on s.chunk_id=c.id",
    `where ${where.join(" and ")}`,
    "group by c.source_id, c.source_ref, c.owner",
    "order by matchScore desc, treeScore desc, chunkCount desc",
    "limit 8;",
  ].join(" ");
  const { stdout } = await execFileAsync("sqlite3", ["-json", MEMORY_TREE_DB_PATH, sql], { timeout: 2500, maxBuffer: 4 * 1024 * 1024 });
  const rows = JSON.parse(stdout || "[]");
  return Array.isArray(rows)
    ? rows.map((row) => ({
        sourceId: String(row.sourceId || ""),
        sourceRef: String(row.sourceRef || ""),
        owner: String(row.owner || ""),
        matchScore: Number(row.matchScore || 0),
        chunkCount: Number(row.chunkCount || 0),
        treeScore: Number(row.treeScore || 0),
      })).filter((row) => row.sourceId || row.sourceRef)
    : [];
}

async function querySourceTreeSummaryHints(terms) {
  if (!terms.length) return [];
  const sql = [
    "select id, tree_id as treeId, level, content, topics_json as topics, score",
    "from mem_tree_summaries",
    `where coalesce(deleted,0)=0 and length(content)>0 and (${sourceTreeSummaryMatchWhere(terms)})`,
    "order by score desc, sealed_at_ms desc",
    "limit 4;",
  ].join(" ");
  const { stdout } = await execFileAsync("sqlite3", ["-json", MEMORY_TREE_DB_PATH, sql], { timeout: 2000, maxBuffer: 2 * 1024 * 1024 });
  const rows = JSON.parse(stdout || "[]");
  return Array.isArray(rows)
    ? rows.map((row) => ({
        id: String(row.id || ""),
        treeId: String(row.treeId || ""),
        level: Number(row.level || 0),
        content: String(row.content || ""),
        score: Number(row.score || 0),
      })).filter((row) => row.id || row.content)
    : [];
}

function sourceTreeMatchExpr(alias, terms = []) {
  return terms.map((term) => {
    const safeTerm = quoteSql(term);
    const like = `%${safeTerm}%`;
    return [
      `(case when ${alias}.content like '${like}' then 5 else 0 end)`,
      `(case when ${alias}.source_id like '${like}' then 3 else 0 end)`,
      `(case when ${alias}.source_ref like '${like}' then 2 else 0 end)`,
      `(case when ${alias}.owner like '${like}' then 2 else 0 end)`,
      `(case when ${alias}.tags_json like '${like}' then 1 else 0 end)`,
    ].join(" + ");
  }).join(" + ") || "0";
}

function sourceTreeMatchWhere(alias, terms = []) {
  return terms.map((term) => {
    const safeTerm = quoteSql(term);
    const like = `%${safeTerm}%`;
    return `(${alias}.content like '${like}' or ${alias}.source_id like '${like}' or ${alias}.source_ref like '${like}' or ${alias}.owner like '${like}' or ${alias}.tags_json like '${like}')`;
  }).join(" or ") || "0";
}

function sourceTreeSummaryMatchWhere(terms = []) {
  return terms.map((term) => {
    const safeTerm = quoteSql(term);
    const like = `%${safeTerm}%`;
    return `(content like '${like}' or topics_json like '${like}')`;
  }).join(" or ") || "0";
}

function sourceTreeScopeClauses(controls, alias) {
  const normalized = normalizeSourceControls(controls);
  const clauses = [];
  if (normalized.allowedAuthors.length > 0) {
    clauses.push(`${alias}.owner in (${normalized.allowedAuthors.map((author) => `'${quoteSql(author)}'`).join(",")})`);
  }
  if (normalized.allowedSourceKeys.length > 0) {
    const keys = normalized.allowedSourceKeys.map((key) => `'${quoteSql(key)}'`).join(",");
    clauses.push(`(${alias}.source_id in (${keys}) or ${alias}.source_ref in (${keys}))`);
  }
  if (normalized.excludedSourceKeys.length > 0) {
    const keys = normalized.excludedSourceKeys.map((key) => `'${quoteSql(key)}'`).join(",");
    clauses.push(`not (${alias}.source_id in (${keys}) or ${alias}.source_ref in (${keys}))`);
  }
  return clauses;
}

function workflowNamespaceFor(namespace) {
  const base = String(namespace || "").trim() || DEFAULT_NAMESPACE;
  return base.endsWith("-workflow") ? base : `${base}-workflow`;
}

function sourceNamespaceFor(namespace) {
  const base = String(namespace || "").trim() || DEFAULT_NAMESPACE;
  return base.endsWith("-workflow") ? base.slice(0, -"-workflow".length) || DEFAULT_NAMESPACE : base;
}

function mergeKnowledgeContexts(results = []) {
  const seen = new Set();
  const blocks = [];
  for (const result of results) {
    const text = normalizeContextText(result);
    for (const block of text.split(/\n{2,}/).map((item) => item.trim()).filter(Boolean)) {
      const key = block.replace(/\s+/g, " ").slice(0, 220);
      if (seen.has(key)) continue;
      seen.add(key);
      blocks.push(block);
    }
  }
  return blocks.join("\n\n");
}

async function queryAuthorDiversityFromDb(context, query, authors = []) {
  if (!existsSync(DB_PATH) || authors.length === 0) return "";
  const namespace = quoteSql(sourceNamespaceFor(context.namespace));
  const terms = authorDiversitySearchTerms(query);
  const scoreExpr = terms.length > 0
    ? terms
        .map((term) => {
          const safeTerm = quoteSql(term);
          return `(case when title like '%${safeTerm}%' then 4 else 0 end) + (case when content like '%${safeTerm}%' then 1 else 0 end)`;
        })
        .join(" + ")
    : "0";
  const matchClause = terms.length > 0
    ? `and (${terms
        .map((term) => {
          const safeTerm = quoteSql(term);
          return `title like '%${safeTerm}%' or content like '%${safeTerm}%'`;
        })
        .join(" or ")})`
    : "";
  const blocks = [];

  for (const author of authors) {
    const safeAuthor = quoteSql(author);
    const sql = [
      `select content, (${scoreExpr}) as score from memory_docs`,
      `where namespace='${namespace}' and json_extract(metadata_json,'$.author')='${safeAuthor}' ${matchClause}`,
      `order by score desc, updated_at desc`,
      `limit 2;`,
    ].join(" ");
    try {
      const { stdout } = await execFileAsync("sqlite3", ["-json", DB_PATH, sql], { timeout: 5000, maxBuffer: 8 * 1024 * 1024 });
      const rows = JSON.parse(stdout || "[]");
      for (const row of rows) {
        if (typeof row?.content === "string" && row.content.trim()) blocks.push(row.content.trim());
      }
    } catch (error) {
      if (process.env.AMAZON_QA_DEBUG) {
        console.warn(`Author DB fallback skipped for ${author}: ${friendlyError(error)}`);
      }
    }
  }

  return blocks.join("\n\n");
}

function authorDiversitySearchTerms(query) {
  const value = String(query || "");
  const terms = [];
  const add = (...items) => {
    for (const item of items) {
      const term = String(item || "").trim();
      if (term && !terms.includes(term)) terms.push(term);
    }
  };
  if (/主图|图片|视觉|点击率|转化率/.test(value)) add("主图", "点击率", "转化率", "图片", "视觉", "Listing", "页面");
  if (/广告|推广|投放|acos|cpc/i.test(value)) add("广告", "推广", "投放", "ACOS", "CPC", "关键词");
  if (/listing|文案|关键词|收录|标题|search term/i.test(value)) add("Listing", "文案", "关键词", "收录", "标题", "Search Term");
  if (/选品|产品|值不值得|市场|竞争/.test(value)) add("选品", "产品", "市场", "竞争", "利润", "差异化");
  return terms.slice(0, 10);
}

async function buildSourceContextResponse(context, body = {}) {
  const sessionId = String(body.sessionId || "").trim();
  if (sessionId && !sessions.has(sessionId)) {
    await getSession(context.namespace, sessionId, { create: false });
  }
  const requested = resolveRequestedSource(body);
  const source = requested.source;
  if (!source) {
    return buildSourceContextFromArticle({}, "", { quote: "" });
  }

  const article = await findSourceArticle(context, source);
  const sourceWithMetadata = {
    ...source,
    ...article.metadata,
    title: source.title || article.title || article.metadata.title || "",
    author: source.author || article.metadata.author || "",
    date: source.date || article.metadata.date || "",
    sourcePath: source.sourcePath || article.metadata.source_path || article.metadata.sourcePath || "",
    sourceUrl: source.sourceUrl || article.metadata.source_url || article.metadata.sourceUrl || "",
  };
  return buildSourceContextFromArticle(sourceWithMetadata, article.content || "", {
    quote: requested.quote || source.excerpt || "",
  });
}

function resolveRequestedSource(body = {}) {
  const sessionId = String(body.sessionId || "").trim();
  const messageIndex = Number(body.messageIndex);
  const sourceIndex = Number(body.sourceIndex);
  const quote = String(body.quote || "").trim();
  const session = sessionId ? sessions.get(sessionId) : null;
  const message = Number.isInteger(messageIndex) ? session?.history?.[messageIndex] : null;
  const sessionSource = Number.isInteger(sourceIndex) ? message?.sources?.[sourceIndex] : null;
  const source = normalizeSourceRequest(sessionSource || body.source);
  return { source, quote };
}

function normalizeSourceRequest(source) {
  if (!source || typeof source !== "object") return null;
  return {
    author: safeSourceText(source.author, 80),
    date: safeSourceText(source.date, 24),
    title: safeSourceText(source.title || source.label, 220),
    sourceUrl: safeSourceText(source.sourceUrl || source.url, 700),
    sourcePath: safeSourceText(source.sourcePath || source.path, 700),
    excerpt: safeSourceText(source.excerpt || source.quote, 1800),
  };
}

function safeSourceText(value, max = 240) {
  return String(value || "").trim().slice(0, max);
}

async function findSourceArticle(context, source) {
  const fromUserSource = await findUserSourceArticle(context.namespace, source);
  if (fromUserSource) return fromUserSource;
  const fromDb = await findSourceArticleInMemoryDb(context, source);
  if (fromDb) return fromDb;
  return {
    content: "",
    metadata: {
      author: source.author || "",
      date: source.date || "",
      title: source.title || "",
      source_path: source.sourcePath || "",
      source_url: source.sourceUrl || "",
    },
  };
}

async function findUserSourceArticle(namespace, source) {
  const id = userSourceIdFromSourceRequest(source);
  if (!id) return null;
  const record = await readUserSourceIfExists(namespace, id);
  if (!record?.content) return null;
  return {
    title: record.title,
    content: userSourceContextBlock(record),
    metadata: {
      author: USER_SOURCE_AUTHOR,
      date: String(record.updatedAt || record.createdAt || "").slice(0, 10),
      title: record.title,
      source_path: `user-sources/${record.id}.json`,
      source_url: `user-source://${record.id}`,
    },
  };
}

function userSourceIdFromSourceRequest(source) {
  const values = [source?.sourceUrl, source?.sourcePath].map((value) => String(value || ""));
  for (const value of values) {
    const match = value.match(/user-source:\/\/([a-zA-Z0-9_.-]+)/) || value.match(/user-sources\/([a-zA-Z0-9_.-]+)\.json/);
    if (match) return match[1];
  }
  return "";
}

async function findSourceArticleInMemoryDb(context, source) {
  if (!existsSync(DB_PATH)) return null;
  const namespace = quoteSql(sourceNamespaceFor(context.namespace));
  const conditions = sourceArticleSqlConditions(source);
  if (conditions.length === 0) return null;
  const order = sourceArticleSqlOrder(source);
  const sql = [
    "select key,title,content,metadata_json from memory_docs",
    `where namespace='${namespace}' and (${conditions.join(" or ")})`,
    `order by ${order}`,
    "limit 1;",
  ].join(" ");
  try {
    const { stdout } = await execFileAsync("sqlite3", ["-json", DB_PATH, sql], { timeout: 3000, maxBuffer: 2 * 1024 * 1024 });
    const rows = JSON.parse(stdout || "[]");
    const row = rows[0];
    if (!row) return null;
    return {
      content: row.content || "",
      title: row.title || "",
      key: row.key || "",
      metadata: safeJsonObject(row.metadata_json),
    };
  } catch (error) {
    if (process.env.AMAZON_QA_DEBUG) {
      console.warn(`Source context lookup skipped: ${friendlyError(error)}`);
    }
    return null;
  }
}

function safeJsonObject(value) {
  try {
    const parsed = JSON.parse(value || "{}");
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

function sourceArticleSqlConditions(source) {
  const conditions = [];
  if (source.sourcePath) {
    conditions.push(`json_extract(metadata_json,'$.source_path')='${quoteSql(source.sourcePath)}'`);
    conditions.push(`json_extract(metadata_json,'$.sourcePath')='${quoteSql(source.sourcePath)}'`);
  }
  if (source.sourceUrl) {
    conditions.push(`json_extract(metadata_json,'$.source_url')='${quoteSql(source.sourceUrl)}'`);
    conditions.push(`json_extract(metadata_json,'$.sourceUrl')='${quoteSql(source.sourceUrl)}'`);
  }
  if (source.author && source.title) {
    const titleLike = `%${quoteSql(source.title)}%`;
    conditions.push(`(title like '${titleLike}' and json_extract(metadata_json,'$.author')='${quoteSql(source.author)}')`);
  }
  if (source.author && source.date && source.title) {
    const keyLike = `%/${quoteSql(source.author)}/${quoteSql(source.date.slice(0, 10))}_%${quoteSql(source.title)}%`;
    conditions.push(`key like '${keyLike}'`);
  }
  return conditions;
}

function sourceArticleSqlOrder(source) {
  const clauses = [];
  if (source.sourcePath) {
    clauses.push(`when json_extract(metadata_json,'$.source_path')='${quoteSql(source.sourcePath)}' then 0`);
    clauses.push(`when json_extract(metadata_json,'$.sourcePath')='${quoteSql(source.sourcePath)}' then 0`);
  }
  if (source.sourceUrl) {
    clauses.push(`when json_extract(metadata_json,'$.source_url')='${quoteSql(source.sourceUrl)}' then 1`);
    clauses.push(`when json_extract(metadata_json,'$.sourceUrl')='${quoteSql(source.sourceUrl)}' then 1`);
  }
  if (source.author && source.date && source.title) {
    clauses.push(`when key like '%/${quoteSql(source.author)}/${quoteSql(source.date.slice(0, 10))}_%${quoteSql(source.title)}%' then 2`);
  }
  if (source.author && source.title) {
    clauses.push(`when title like '%${quoteSql(source.title)}%' and json_extract(metadata_json,'$.author')='${quoteSql(source.author)}' then 3`);
  }
  return `case ${clauses.join(" ")} else 9 end, updated_at desc`;
}

async function queryNamespace(context, namespace, query, limit, options = {}) {
  try {
    return await rpcCall(context.rpcUrl, context.token, "openhuman.memory_query_namespace", {
      namespace,
      query,
      include_references: true,
      limit,
    });
  } catch (error) {
    if (isFetchFailure(error) && context.coreBaseUrl) {
      await ensureCoreRunning({ coreBaseUrl: context.coreBaseUrl, corePort: context.corePort || DEFAULT_CORE_PORT, token: context.token });
      return rpcCall(context.rpcUrl, context.token, "openhuman.memory_query_namespace", {
        namespace,
        query,
        include_references: true,
        limit,
      });
    }
    if (options.fallback === false) throw error;
    if (process.env.AMAZON_QA_DEBUG) {
      console.warn(`Structured query failed, falling back to context query: ${friendlyError(error)}`);
    }
    return rpcCall(context.rpcUrl, context.token, "openhuman.memory_context_query", {
      namespace,
      query,
      limit,
    });
  }
}

function isFetchFailure(error) {
  const message = error instanceof Error ? error.message : String(error);
  return message.includes("fetch failed") || message.includes("ECONNREFUSED") || message.includes("connection refused");
}

async function getStatus(context) {
  const rows = await readManifestRows();
  const byAuthor = {};
  for (const row of rows) byAuthor[row.author] = (byAuthor[row.author] || 0) + 1;

  let storedDocuments = 0;
  try {
    storedDocuments = (await readdir(DOCS_DIR)).filter((name) => name.endsWith(".md")).length;
  } catch {
    storedDocuments = 0;
  }

  const health = await getKnowledgeHealth(context.namespace, rows);
  const sourceTreeDrain = await getSourceTreeDrainStatus(health.sourceTree || {});
  const userSources = await listUserSources(context.namespace);
  const learningNotes = await listLearningNotes(context.namespace);
  const status = {
    namespace: context.namespace,
    manifestDocuments: rows.length,
    storedDocuments,
    userSourceCount: userSources.length,
    learningNoteCount: learningNotes.length,
    health,
    sourceTreeDrain,
    byAuthor,
    suggestedQuestions: DEFAULT_SUGGESTED_QUESTIONS,
  };
  return {
    ...status,
    readiness: buildKnowledgeReadinessSummary(status),
  };
}

async function getKnowledgeHealth(namespace, manifestRows = []) {
  if (!existsSync(DB_PATH)) {
    return buildKnowledgeHealthSummary({ sourceTree: await getSourceTreeHealth(manifestRows) });
  }
  const ns = quoteSql(namespace);
  const sql = [
    `select`,
    `(select count(*) from memory_docs where namespace='${ns}') || '|' ||`,
    `(select count(*) from vector_chunks where namespace='${ns}') || '|' ||`,
    `(select count(*) from vector_chunks where namespace='${ns}' and embedding is not null and length(embedding)>0) || '|' ||`,
    `(select count(*) from graph_namespace where namespace='${ns}');`,
  ].join(" ");

  try {
    const { stdout } = await execFileAsync("sqlite3", [DB_PATH, sql], { timeout: 3000 });
    const [documents, chunks, embeddedChunks, graphRelations] = stdout
      .trim()
      .split("|")
      .map((value) => Number(value || 0));
    return buildKnowledgeHealthSummary({
      documents,
      chunks,
      embeddedChunks,
      graphRelations,
      sourceTree: await getSourceTreeHealth(manifestRows),
    });
  } catch {
    return buildKnowledgeHealthSummary({ sourceTree: await getSourceTreeHealth(manifestRows) });
  }
}

async function getSourceTreeHealth(manifestRows = []) {
  if (!existsSync(MEMORY_TREE_DB_PATH)) {
    return summarizeSourceTreeStatus({ manifestRows, stats: {} });
  }
  try {
    const [counts, chunkSourceIds, ingestedSourceIds, jobStatusRows] = await Promise.all([
      sqliteMemoryTreeLines(
        "select " +
          "(select count(*) from mem_tree_chunks) || '|' || " +
          "(select count(*) from mem_tree_trees) || '|' || " +
          "(select count(*) from mem_tree_summaries);",
      ),
      sqliteMemoryTreeLines("select distinct source_id from mem_tree_chunks where source_kind='document' order by source_id;"),
      sqliteMemoryTreeLines("select source_id from mem_tree_ingested_sources where source_kind='document' order by source_id;"),
      sqliteMemoryTreeLines("select status || '|' || count(*) from mem_tree_jobs group by status;"),
    ]);
    const [chunks, trees, summaries] = String(counts[0] || "")
      .split("|")
      .map((value) => Number(value || 0));
    return summarizeSourceTreeStatus({
      manifestRows,
      stats: { chunks, trees, summaries, chunkSourceIds, ingestedSourceIds, ...parseSourceTreeJobStatusRows(jobStatusRows) },
    });
  } catch {
    return summarizeSourceTreeStatus({ manifestRows, stats: {} });
  }
}

function parseSourceTreeJobStatusRows(rows = []) {
  const counts = { readyJobs: 0, runningJobs: 0, failedJobs: 0, doneJobs: 0 };
  for (const row of rows) {
    const [status, countText] = String(row || "").split("|");
    const count = Math.max(0, Number(countText || 0));
    if (status === "ready") counts.readyJobs = count;
    if (status === "running") counts.runningJobs = count;
    if (status === "failed") counts.failedJobs = count;
    if (status === "done") counts.doneJobs = count;
  }
  return counts;
}

async function sqliteMemoryTreeLines(sql) {
  const { stdout } = await execFileAsync("sqlite3", ["-cmd", ".timeout 15000", MEMORY_TREE_DB_PATH, sql], { timeout: 18000 });
  return stdout
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}

function quoteSql(value) {
  return String(value).replace(/'/g, "''");
}

async function readManifestRows() {
  const text = await readFile(MANIFEST_PATH, "utf8");
  return text
    .split("\n")
    .filter((line) => line.trim())
    .map((line) => JSON.parse(line));
}

async function readJsonBody(request) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > 2 * 1024 * 1024) throw new Error("Request body is too large");
    chunks.push(chunk);
  }
  const text = Buffer.concat(chunks).toString("utf8");
  return text ? JSON.parse(text) : {};
}

function sendJson(response, statusCode, payload) {
  sendText(response, statusCode, JSON.stringify(payload, null, 2), "application/json; charset=utf-8");
}

function sendText(response, statusCode, text, contentType) {
  response.writeHead(statusCode, {
    "content-type": contentType,
    "cache-control": "no-store",
  });
  response.end(text);
}

function friendlyError(error) {
  const message = error instanceof Error ? error.message : String(error);
  if (message.includes("memory_context_query") || message.includes("memory_query_namespace")) {
    return "本地知识库查询失败，请确认 OpenHuman 已正常启动。";
  }
  if (message.includes("not built")) return "OpenHuman 还没有构建成功，请先完成本地安装。";
  return message;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

main().catch((error) => {
  console.error(friendlyError(error));
  if (coreProcess && !coreProcess.killed) coreProcess.kill("SIGTERM");
  process.exit(1);
});
