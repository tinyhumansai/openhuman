#!/usr/bin/env node

import { existsSync, readFileSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { execFile, spawn } from "node:child_process";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";

import { runAmazonQaFinalAcceptance, runAmazonQaSmoke } from "./amazon-qa-e2e-smoke.mjs";
import { resolveAmazonQaPaths } from "./amazon-qa-paths.mjs";
import { buildSourceTreeDrainPreflight } from "./amazon-source-tree-lib.mjs";

const QA_PATHS = resolveAmazonQaPaths(import.meta.dirname);
const ROOT = QA_PATHS.root;
const WORKSPACE = QA_PATHS.workspace;
const OPENHUMAN_REPO = QA_PATHS.repoRoot;
const RUN_DIR = QA_PATHS.runRoot;
const LOG_PATH = path.join(RUN_DIR, "logs", "amazon-qa-server-7790.log");
const CONFIG_PATH = QA_PATHS.configPath;
const MANIFEST_PATH = QA_PATHS.manifestPath;
const MEMORY_DB_PATH = QA_PATHS.memoryDbPath;
const MEMORY_TREE_DB_PATH = QA_PATHS.memoryTreeDbPath;
const CORE_BIN = QA_PATHS.coreBin;
const UI_PATH = QA_PATHS.uiPath;
const SERVER_PATH = QA_PATHS.serverPath;
const HANDOFF_PATH = QA_PATHS.handoffPath;
const ACCEPTANCE_EVIDENCE_PATH = path.join(QA_PATHS.outputRoot, "amazon-learning-qa-acceptance-evidence.json");
const DEFAULT_HOST = "127.0.0.1";
const DEFAULT_PORT = 7790;
const DEFAULT_CORE_PORT = 7789;
const DEFAULT_BASE_URL = `http://${DEFAULT_HOST}:${DEFAULT_PORT}`;
const DEFAULT_OLLAMA_ENDPOINT = "http://127.0.0.1:11434";
const EXPECTED_DOCUMENTS = 1779;
const EXPECTED_CHUNKS = 14597;
const PRODUCT_SCRIPT = path.relative(process.cwd(), path.join(import.meta.dirname, "amazon-qa-product.mjs")) || path.join(import.meta.dirname, "amazon-qa-product.mjs");
const PRODUCT_COMMAND = `node ${PRODUCT_SCRIPT}`;
const execFileAsync = promisify(execFile);

function usage() {
  console.log(`Usage:
  ${PRODUCT_COMMAND} doctor [--json]
  ${PRODUCT_COMMAND} start [--port ${DEFAULT_PORT}] [--core-port ${DEFAULT_CORE_PORT}]
  ${PRODUCT_COMMAND} status [--json]
  ${PRODUCT_COMMAND} smoke [--base-url ${DEFAULT_BASE_URL}]
  ${PRODUCT_COMMAND} acceptance [--base-url ${DEFAULT_BASE_URL}]
  ${PRODUCT_COMMAND} acceptance-evidence [--base-url ${DEFAULT_BASE_URL}]
  ${PRODUCT_COMMAND} completion-audit [--json]
  ${PRODUCT_COMMAND} handoff

Purpose:
  One local delivery entry for the Amazon learning Q&A product: checks local files,
  Ollama models, service readiness, semantic index coverage, and deployment limits.`);
}

function argValue(args, name, fallback = undefined) {
  const index = args.indexOf(name);
  if (index === -1) return fallback;
  const value = args[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`Missing value for ${name}`);
  return value;
}

async function main() {
  const [command = "doctor", ...args] = process.argv.slice(2);
  if (command === "--help" || command === "-h" || command === "help") {
    usage();
    return;
  }
  if (command === "doctor") return printDoctor(args);
  if (command === "start") return startProduct(args);
  if (command === "status") return printStatus(args);
  if (command === "smoke") return runSmoke(args);
  if (command === "acceptance") return runAcceptance(args);
  if (command === "acceptance-evidence") return writeAcceptanceEvidence(args);
  if (command === "completion-audit" || command === "audit") return printCompletionAudit(args);
  if (command === "handoff") return writeHandoff();
  throw new Error(`Unknown command: ${command}`);
}

export async function buildProductDoctorReport(options = {}) {
  const baseUrl = String(options.baseUrl || DEFAULT_BASE_URL).replace(/\/+$/, "");
  const paths = localPathChecks();
  const [service, localAi, git] = await Promise.all([
    fetchStatus(baseUrl),
    checkOllama(options.ollamaEndpoint || DEFAULT_OLLAMA_ENDPOINT),
    gitStatus(),
  ]);
  const health = service.status?.health || {};
  const readiness = service.status?.readiness || {};
  const drain = service.status?.sourceTreeDrain || {};
  const documents = Number(health.documents || 0);
  const chunks = Number(health.chunks || 0);
  const embedded = Number(health.embeddedChunks || 0);
  const coverage = Number(health.vectorCoveragePercent || 0);
  const userSourceCount = Number(service.status?.userSourceCount || 0);
  const learningNoteCount = Number(service.status?.learningNoteCount || 0);
  const critical = [];
  const warnings = [];

  if (!paths.coreBinary.exists) critical.push("OpenHuman 核心程序不存在，需要先构建 openhuman-core。");
  if (!paths.memoryDatabase.exists) critical.push("本地知识库数据库不存在。");
  if (!paths.manifest.exists) critical.push("作者资料清单不存在。");
  if (!service.ok) critical.push("本地问答服务未运行。");
  if (service.ok && documents !== EXPECTED_DOCUMENTS) critical.push(`资料数异常：${documents}/${EXPECTED_DOCUMENTS}。`);
  if (service.ok && chunks !== EXPECTED_CHUNKS) critical.push(`片段数异常：${chunks}/${EXPECTED_CHUNKS}。`);
  if (service.ok && embedded !== chunks) critical.push(`语义索引未满：${embedded}/${chunks}。`);
  if (!localAi.preflight.ok) warnings.push(localAi.preflight.message);
  if (service.ok && readiness.answerStatus !== "ready") critical.push("问答和引用未达到就绪状态。");
  if (service.ok && readiness.learningStatus !== "ready") {
    warnings.push("来源树仍在后台深加工；这不影响问答和引用，但还不是完整结构化学习状态。");
  }
  if (!git.productSourceInRepo) {
    warnings.push("当前运行的是外层本地问答入口；如要提交到 openhuman 仓库，请使用仓库内 tools/amazon-learning-qa 产品包。");
  }

  const ok = critical.length === 0;
  return {
    ok,
    level: ok ? (warnings.length ? "usable_with_warnings" : "ready") : "needs_action",
    url: baseUrl,
    generatedAt: new Date().toISOString(),
    service: {
      ok: service.ok,
      error: service.error || "",
      documents,
      userSourceCount,
      learningNoteCount,
      chunks,
      embedded,
      coverage,
      readinessLevel: readiness.level || "",
      answerStatus: readiness.answerStatus || "",
      learningStatus: readiness.learningStatus || "",
      sourceTreeDrainState: drain.state || drain.level || "",
      sourceTreeDrainMessage: drain.message || "",
      sourceTreeProcessedJobs: Number(drain.processedJobs || 0),
      sourceTreeQueuedJobs: Number(drain.queuedJobs || 0),
      sourceTreeReadyJobs: Number(drain.readyJobs || 0),
      sourceTreeRunningJobs: Number(drain.runningJobs || 0),
      sourceTreeDoneJobs: Number(drain.doneJobs || 0),
      sourceTreeFailedJobs: Number(drain.failedJobs || 0),
      sourceTreeJobsPerMinute: Number(drain.jobsPerMinute || 0),
      sourceTreeEstimatedMinutesRemaining: Number(drain.estimatedMinutesRemaining || 0),
      sourceTreeEstimatedRemainingText: formatMinutes(drain.estimatedMinutesRemaining),
    },
    localAi,
    paths,
    git,
    deployment: {
      vercelReady: false,
      reason: "当前产品依赖本地 SQLite、Ollama、本地 openhuman-core 长驻进程和本机文件路径，不能原样部署到 Vercel。",
      realisticTarget: "本地 Mac/Linux 主机或 VPS 长驻服务；如需 Vercel，需要先把本地数据库、模型和核心程序迁到云端服务。",
    },
    critical,
    warnings: unique(warnings),
    nextActions: nextActions({ ok, service, localAi, readiness, drain, git }),
  };
}

function localPathChecks() {
  return {
    root: { path: ROOT, exists: existsSync(ROOT) },
    config: { path: CONFIG_PATH, exists: existsSync(CONFIG_PATH) },
    manifest: { path: MANIFEST_PATH, exists: existsSync(MANIFEST_PATH) },
    memoryDatabase: { path: MEMORY_DB_PATH, exists: existsSync(MEMORY_DB_PATH) },
    memoryTreeDatabase: { path: MEMORY_TREE_DB_PATH, exists: existsSync(MEMORY_TREE_DB_PATH) },
    coreBinary: { path: CORE_BIN, exists: existsSync(CORE_BIN) },
    server: { path: SERVER_PATH, exists: existsSync(SERVER_PATH) },
    ui: { path: UI_PATH, exists: existsSync(UI_PATH) },
  };
}

async function fetchStatus(baseUrl) {
  try {
    const response = await fetch(`${baseUrl}/api/status`, { signal: AbortSignal.timeout(5000) });
    if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
    return { ok: true, status: await response.json() };
  } catch (error) {
    return { ok: false, error: error.message, status: null };
  }
}

async function checkOllama(endpoint) {
  const { requiredModels, optionalModels } = await configuredModels();
  try {
    const response = await fetch(`${endpoint.replace(/\/+$/, "")}/api/tags`, { signal: AbortSignal.timeout(5000) });
    if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
    const payload = await response.json();
    const availableModels = Array.isArray(payload.models) ? payload.models.map((model) => model.name).filter(Boolean) : [];
    const availableSet = new Set(availableModels);
    return {
      endpoint,
      optionalModels,
      optionalMissingModels: optionalModels.filter((model) => !availableSet.has(model)),
      preflight: buildSourceTreeDrainPreflight({ ok: true, endpoint, requiredModels, availableModels }),
    };
  } catch (error) {
    return {
      endpoint,
      optionalModels,
      optionalMissingModels: optionalModels,
      preflight: buildSourceTreeDrainPreflight({ ok: false, endpoint, requiredModels, error: error.message }),
    };
  }
}

async function configuredModels() {
  if (!existsSync(CONFIG_PATH)) return { requiredModels: ["mxbai-embed-large:latest"], optionalModels: [] };
  const text = await readFile(CONFIG_PATH, "utf8");
  const requiredModels = unique([
    ...tomlSectionValues(text, "memory", ["embedding_model"]),
    ...tomlSectionValues(text, "local_ai", ["embedding_model_id"]),
    ...tomlSectionValues(text, "memory_tree", [
      "embedding_model",
      "llm_extractor_model",
      "llm_summariser_model",
      "llm_summarizer_model",
    ]),
    "mxbai-embed-large:latest",
  ]);
  const optionalModels = unique(tomlSectionValues(text, "local_ai", ["chat_model_id"]));
  return { requiredModels, optionalModels };
}

function tomlSectionValues(text, sectionName, keys) {
  const lines = String(text || "").split(/\r?\n/);
  let current = "";
  const values = [];
  for (const line of lines) {
    const section = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (section) {
      current = section[1].trim();
      continue;
    }
    if (current !== sectionName) continue;
    const match = line.match(/^\s*([A-Za-z0-9_.-]+)\s*=\s*(.+?)\s*$/);
    if (!match || !keys.includes(match[1])) continue;
    values.push(unquoteToml(match[2]));
  }
  return values.filter(Boolean);
}

function unquoteToml(value) {
  return String(value || "").trim().replace(/^["']|["']$/g, "").trim();
}

async function gitStatus() {
  const outerIsRepo = await isGitRepo(ROOT);
  const repoPath = OPENHUMAN_REPO;
  const innerIsRepo = await isGitRepo(repoPath);
  let remote = "";
  if (innerIsRepo) {
    try {
      const { stdout } = await execFileAsync("git", ["-C", repoPath, "remote", "get-url", "origin"], { timeout: 3000 });
      remote = stdout.trim();
    } catch {
      remote = "";
    }
  }
  return { outerIsRepo, repoPath, innerIsRepo, remote, productSourceInRepo: QA_PATHS.productSourceInRepo };
}

async function isGitRepo(cwd) {
  try {
    const { stdout } = await execFileAsync("git", ["-C", cwd, "rev-parse", "--is-inside-work-tree"], { timeout: 3000 });
    return stdout.trim() === "true";
  } catch {
    return false;
  }
}

function nextActions({ ok, service, localAi, readiness, drain, git }) {
  const actions = [];
  if (!service.ok) actions.push(`运行 ${PRODUCT_COMMAND} start 启动本地入口。`);
  if (!localAi.preflight.ok) actions.push("先启动 Ollama 并确认所需模型已安装。");
  if (service.ok && readiness.learningStatus !== "ready" && Number(drain.queuedJobs || 0) > 0) {
    actions.push("在页面按 10/50/250 的有限批次继续跑来源树深加工；每批跑完会自动暂停，问答可以先正常使用。");
  }
  if (!git.productSourceInRepo) {
    actions.push("如要提交发布，先使用仓库内 tools/amazon-learning-qa 产品包，避免把外层本地数据库和资料误提交。");
  }
  if (ok) actions.push(`运行 ${PRODUCT_COMMAND} acceptance 做终版真实问答、追问和换题不串题验收。`);
  return unique(actions);
}

async function printDoctor(args) {
  const report = await buildProductDoctorReport();
  if (args.includes("--json")) {
    console.log(JSON.stringify(report, null, 2));
    return;
  }
  printHumanReport(report);
  if (!report.ok) process.exitCode = 1;
}

async function printStatus(args) {
  const report = await buildProductDoctorReport();
  if (args.includes("--json")) {
    console.log(JSON.stringify(report.service, null, 2));
    return;
  }
  console.log(`入口：${report.url}`);
  console.log(`状态：${report.service.answerStatus === "ready" ? "可问答" : "未就绪"}，${report.service.learningStatus === "ready" ? "来源树完成" : "来源树增强中"}`);
  console.log(`资料：${report.service.documents} 篇，我的资料 ${report.service.userSourceCount || 0} 份，学习笔记 ${report.service.learningNoteCount || 0} 条，片段 ${report.service.chunks} 个，语义索引 ${report.service.embedded}/${report.service.chunks}。`);
  console.log(`来源树：等待 ${report.service.sourceTreeQueuedJobs}，完成 ${report.service.sourceTreeDoneJobs}，失败 ${report.service.sourceTreeFailedJobs}。`);
  if (report.service.sourceTreeEstimatedRemainingText) {
    console.log(`来源树预计：按最近速度约 ${report.service.sourceTreeEstimatedRemainingText}；不影响当前问答和引用。`);
  }
}

async function startProduct(args) {
  const port = Number(argValue(args, "--port", String(DEFAULT_PORT)));
  const corePort = Number(argValue(args, "--core-port", String(DEFAULT_CORE_PORT)));
  const baseUrl = `http://${DEFAULT_HOST}:${port}`;
  const current = await fetchStatus(baseUrl);
  if (current.ok) {
    console.log(`亚马逊学习问答已经在运行：${baseUrl}`);
    return;
  }
  await mkdir(path.dirname(LOG_PATH), { recursive: true });
  spawn("screen", [
    "-dmS",
    "amazon-qa",
    "bash",
    "-lc",
    `cd ${shellQuote(ROOT)} && node ${shellQuote(SERVER_PATH)} --host ${DEFAULT_HOST} --port ${port} --core-port ${corePort} > ${shellQuote(LOG_PATH)} 2>&1`,
  ], { stdio: "ignore", detached: true }).unref();

  const started = await waitForStatus(baseUrl, 30_000);
  if (!started.ok) {
    console.error(`启动失败：${started.error || "服务没有在 30 秒内就绪"}`);
    console.error(`日志：${LOG_PATH}`);
    process.exitCode = 1;
    return;
  }
  console.log(`亚马逊学习问答已启动：${baseUrl}`);
}

async function waitForStatus(baseUrl, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let last = { ok: false, error: "" };
  while (Date.now() < deadline) {
    last = await fetchStatus(baseUrl);
    if (last.ok) return last;
    await new Promise((resolve) => setTimeout(resolve, 800));
  }
  return last;
}

async function runSmoke(args) {
  const baseUrl = argValue(args, "--base-url", DEFAULT_BASE_URL);
  const result = await runAmazonQaSmoke({ baseUrl });
  console.log(JSON.stringify(result, null, 2));
}

async function runAcceptance(args) {
  const baseUrl = argValue(args, "--base-url", DEFAULT_BASE_URL);
  const result = await runAmazonQaFinalAcceptance({ baseUrl });
  console.log(JSON.stringify(result, null, 2));
}

async function writeAcceptanceEvidence(args) {
  const baseUrl = argValue(args, "--base-url", DEFAULT_BASE_URL);
  const result = await runAmazonQaFinalAcceptance({ baseUrl });
  const evidence = {
    generatedAt: new Date().toISOString(),
    command: `${PRODUCT_COMMAND} acceptance-evidence --base-url ${baseUrl}`,
    baseUrl,
    result,
  };
  await mkdir(path.dirname(ACCEPTANCE_EVIDENCE_PATH), { recursive: true });
  await writeFile(ACCEPTANCE_EVIDENCE_PATH, `${JSON.stringify(evidence, null, 2)}\n`, "utf8");
  console.log(ACCEPTANCE_EVIDENCE_PATH);
}

async function printCompletionAudit(args) {
  const report = await buildProductDoctorReport();
  const audit = buildCompletionAudit(report);
  if (args.includes("--json")) {
    console.log(JSON.stringify(audit, null, 2));
    return;
  }
  console.log(completionAuditMarkdown(audit));
}

async function writeHandoff() {
  const report = await buildProductDoctorReport();
  await mkdir(path.dirname(HANDOFF_PATH), { recursive: true });
  await writeFile(HANDOFF_PATH, handoffMarkdown(report), "utf8");
  console.log(HANDOFF_PATH);
}

export function buildCompletionAudit(report) {
  const acceptanceEvidence = Object.hasOwn(report, "acceptanceEvidence")
    ? report.acceptanceEvidence
    : readAcceptanceEvidence();
  const acceptanceCheck = validateAcceptanceEvidence(acceptanceEvidence);
  const semanticReady = report.ok
    && report.service?.answerStatus === "ready"
    && Number(report.service?.documents || 0) === EXPECTED_DOCUMENTS
    && Number(report.service?.chunks || 0) === EXPECTED_CHUNKS
    && Number(report.service?.embedded || 0) === Number(report.service?.chunks || 0)
    && Number(report.service?.coverage || 0) >= 99;
  const sourceTreeReady = report.service?.learningStatus === "ready" && Number(report.service?.sourceTreeQueuedJobs || 0) === 0;
  const localPackageExists = existsSync(SERVER_PATH) && existsSync(UI_PATH);
  const vercelEntryExists = existsSync(path.join(import.meta.dirname, "vercel-entry", "index.html"));
  const requirements = [
    {
      id: "local_semantic_knowledge_base",
      label: "本地语义知识库",
      status: semanticReady ? "proved" : "not_complete",
      evidence: semanticReady
        ? `${report.service.documents} 篇资料、${report.service.chunks} 个片段，语义索引 ${report.service.embedded}/${report.service.chunks}，覆盖率 ${report.service.coverage}%。`
        : "资料数、片段数或语义索引没有达到交付门槛。",
    },
    {
      id: "interactive_memory_qa",
      label: "连续问答、来源引用和图谱入口",
      status: report.service?.answerStatus === "ready"
        ? (acceptanceCheck.ok ? "proved" : "needs_acceptance_evidence")
        : "not_complete",
      evidence: report.service?.answerStatus !== "ready"
        ? "问答服务当前未就绪。"
        : acceptanceCheck.ok
          ? `真实问题验收证据已保存：${acceptanceCheck.generatedAt}，覆盖 ${acceptanceCheck.scenarioCount} 个场景、换题不串题和结果反馈再追问。`
          : `问答服务当前为 ready；需要运行 ${PRODUCT_COMMAND} acceptance-evidence 作为真实问题验收证据。${acceptanceCheck.reason ? `当前证据问题：${acceptanceCheck.reason}` : ""}`,
    },
    {
      id: "source_tree_learning_layer",
      label: "完整来源树学习层",
      status: sourceTreeReady ? "proved" : "not_complete",
      evidence: sourceTreeReady
        ? "来源树后台深加工已完成。"
        : `来源树仍有 ${Number(report.service?.sourceTreeQueuedJobs || 0)} 个后台任务，已完成 ${Number(report.service?.sourceTreeDoneJobs || 0)} 个，失败 ${Number(report.service?.sourceTreeFailedJobs || 0)} 个；预计剩余 ${report.service?.sourceTreeEstimatedRemainingText || "暂无可靠估计"}。`,
    },
    {
      id: "openhuman_boundary",
      label: "保持 OpenHuman 本地优先边界",
      status: localPackageExists ? "proved" : "not_complete",
      evidence: localPackageExists
        ? "产品包位于仓库 tools/amazon-learning-qa，继续依赖本地 OpenHuman、SQLite 和 Ollama。"
        : "本地产品入口文件缺失。",
    },
    {
      id: "vercel_delivery",
      label: "Vercel 远程交付形态",
      status: vercelEntryExists && report.deployment?.vercelReady === false ? "boundary_only" : "not_complete",
      evidence: vercelEntryExists
        ? "已提供 Vercel 静态交付页，但完整问答不能原样部署到 Vercel Serverless。"
        : "缺少 Vercel 静态交付入口。",
    },
    {
      id: "no_audio_video",
      label: "不做音视频开发",
      status: "proved",
      evidence: "交付说明和远程入口均明确不包含音频或视频功能。",
    },
  ];
  const blocking = requirements.filter((item) => item.status === "not_complete");
  const needsAcceptance = requirements.filter((item) => item.status === "needs_acceptance_evidence");
  const boundaryOnly = requirements.filter((item) => item.status === "boundary_only");
  const completionStatus = blocking.length === 0 && needsAcceptance.length === 0 && boundaryOnly.length === 0
    ? "complete"
    : semanticReady
      ? "local_qa_ready_not_full_final"
      : "needs_action";
  const missingSummary = completionStatus === "complete"
    ? "所有终版要求已有当前证据证明。"
    : `本地语义问答已达到可用状态，但完整终版仍缺少${completionMissingParts({ blocking, needsAcceptance, boundaryOnly }).join("、")}。`;
  return {
    generatedAt: report.generatedAt || new Date().toISOString(),
    completionStatus,
    canMarkGoalComplete: completionStatus === "complete",
    summary: missingSummary,
    requirements,
    blocking: blocking.map((item) => item.id),
    needsAcceptance: needsAcceptance.map((item) => item.id),
    boundaryOnly: boundaryOnly.map((item) => item.id),
    acceptanceEvidence: acceptanceCheck,
    nextActions: [
      ...needsAcceptance.map(() => `运行 ${PRODUCT_COMMAND} acceptance-evidence，并保存输出作为真实问题验收证据。`),
      ...blocking.map((item) => item.id === "source_tree_learning_layer"
        ? "按页面有限批次继续来源树深加工；问答可先正常使用。"
        : item.evidence),
      ...boundaryOnly.map(() => "如要云端完整问答，需要迁移 SQLite、模型服务和 openhuman-core 到可长驻云端服务。"),
    ],
  };
}

function completionMissingParts({ blocking, needsAcceptance, boundaryOnly }) {
  const parts = [];
  if (blocking.some((item) => item.id === "source_tree_learning_layer")) parts.push("来源树完成");
  const otherBlocking = blocking.filter((item) => item.id !== "source_tree_learning_layer").map((item) => item.label);
  parts.push(...otherBlocking);
  if (needsAcceptance.length > 0) parts.push("真实问题验收证据");
  if (boundaryOnly.length > 0) parts.push("云端完整部署条件");
  return parts.length ? parts : ["未归类的终版证据"];
}

function readAcceptanceEvidence() {
  if (!existsSync(ACCEPTANCE_EVIDENCE_PATH)) return null;
  try {
    return JSON.parse(readFileSync(ACCEPTANCE_EVIDENCE_PATH, "utf8"));
  } catch (error) {
    return { error: error.message };
  }
}

function validateAcceptanceEvidence(evidence) {
  if (!evidence || typeof evidence !== "object") return { ok: false, reason: "没有验收证据文件。" };
  if (evidence.error) return { ok: false, reason: evidence.error };
  const result = evidence.result || evidence;
  const scenarios = Array.isArray(result.scenarios) ? result.scenarios : [];
  const topicRows = Array.isArray(result.topicSwitch?.standaloneResults) ? result.topicSwitch.standaloneResults : [];
  const confirmation = result.confirmationLoop || {};
  const ok = result.ok === true
    && Number(result.documents || 0) === EXPECTED_DOCUMENTS
    && Number(result.chunks || 0) === EXPECTED_CHUNKS
    && Number(result.embeddedChunks || 0) === EXPECTED_CHUNKS
    && Number(result.vectorCoveragePercent || 0) >= 99
    && scenarios.length >= 3
    && scenarios.every((item) => Number(item.sources || 0) > 0 && Number(item.graphNodes || 0) > 0)
    && topicRows.length >= 4
    && topicRows.some((item) => item.id === "product-title" && Number(item.sources || 0) > 0)
    && topicRows.some((item) => item.id === "listing-prep" && Number(item.sources || 0) > 0)
    && topicRows.some((item) => item.id === "selection-methods" && Number(item.sources || 0) > 0)
    && topicRows.some((item) => item.id === "persona" && Number(item.graphNodes || 0) > 0)
    && confirmation.status === "needs_source"
    && Number(confirmation.followUpSources || 0) > 0
    && Number(result.studyPackSources || 0) > 0
    && Number(result.studioFlashcards || 0) > 0
    && Number(result.studioMindMapNodes || 0) > 0;
  return {
    ok,
    generatedAt: evidence.generatedAt || "",
    scenarioCount: scenarios.length,
    topicSwitchCount: topicRows.length,
    reason: ok ? "" : "验收输出缺少真实问题、来源、图谱、换题或结果反馈闭环证据。",
  };
}

export function completionAuditMarkdown(audit) {
  const lines = [
    "# 亚马逊学习问答终版完成审计",
    "",
    `生成时间：${audit.generatedAt}`,
    `总体状态：${audit.canMarkGoalComplete ? "可以标记完成" : "尚未达到完整终版"}`,
    "",
    audit.summary,
    "",
    "## 逐项审计",
    "",
  ];
  for (const item of audit.requirements || []) {
    lines.push(`- ${completionStatusLabel(item.status)} ${item.label}：${item.evidence}`);
  }
  if (audit.nextActions?.length) {
    lines.push("", "## 下一步", "");
    audit.nextActions.forEach((item) => lines.push(`- ${item}`));
  }
  return lines.join("\n");
}

function completionStatusLabel(status) {
  if (status === "proved") return "已证明";
  if (status === "needs_acceptance_evidence") return "需验收";
  if (status === "boundary_only") return "仅边界交付";
  return "未完成";
}

export function handoffMarkdown(report) {
  const deliveryStatus = report.ok
    ? (report.warnings?.length ? "本机问答验收通过（仍有提醒）" : "本机验收通过")
    : "未通过本机验收";
  const capabilityTitle = report.ok ? "本机已验证能力" : "当前可用能力（未通过完整验收）";
  const learningState = report.service.learningStatus === "ready"
    ? "完整学习层已就绪"
    : `完整来源树学习仍在处理中：等待 ${report.service.sourceTreeQueuedJobs}，完成 ${report.service.sourceTreeDoneJobs}，失败 ${report.service.sourceTreeFailedJobs}`;
  const learningEta = report.service.sourceTreeEstimatedRemainingText
    ? `按最近速度约 ${report.service.sourceTreeEstimatedRemainingText}，建议继续有限批次处理，不要一次性无边界运行`
    : "暂无可靠耗时估计，以下一批实际速度为准";
  return `# 亚马逊学习问答交付说明

生成时间：${report.generatedAt}

## 交付边界先说明

- 验收状态：${deliveryStatus}
- 运行形态：本机入口 ${report.url}，不是 Vercel 线上部署
- 部署限制：当前不能原样部署到 Vercel。原因：${report.deployment.reason}
- 现实交付方式：${report.deployment.realisticTarget}
- 学习状态：${learningState}
- 来源树预计：${learningEta}
- 音视频：本次不包含音频或视频录制、转写、播放、剪辑、生成、上传功能

## 当前状态

- 本机入口：${report.url}
- 问答状态：${report.service.answerStatus === "ready" ? "可问答、可引用" : "未完全就绪"}
- 资料：${report.service.documents} 篇
- 我的资料：${report.service.userSourceCount || 0} 份
- 学习笔记：${report.service.learningNoteCount || 0} 条
- 片段：${report.service.chunks} 个
- 本地语义索引：${report.service.embedded}/${report.service.chunks}，覆盖率 ${report.service.coverage}%
- 来源树深加工：等待 ${report.service.sourceTreeQueuedJobs}，完成 ${report.service.sourceTreeDoneJobs}，失败 ${report.service.sourceTreeFailedJobs}
- 来源树速度：${report.service.sourceTreeJobsPerMinute ? `${report.service.sourceTreeJobsPerMinute} 个/分钟` : "暂无稳定速度"}；预计剩余 ${report.service.sourceTreeEstimatedRemainingText || "暂无可靠估计"}

## 常用命令

\`\`\`bash
${PRODUCT_COMMAND} doctor
${PRODUCT_COMMAND} start
${PRODUCT_COMMAND} smoke
${PRODUCT_COMMAND} acceptance
${PRODUCT_COMMAND} handoff
\`\`\`

## ${capabilityTitle}

- 本机入口：继续使用 ${report.url}
- 本地语义问答：${report.service.documents} 篇资料、${report.service.chunks} 个片段，语义索引 ${report.service.embedded}/${report.service.chunks}
- 本地模型主回答：优先使用本机 Ollama 生成来源绑定回答，模型不可用或超时时自动回退到稳定模板
- 我的资料：可以粘贴自己的资料，勾选后只围绕这些资料问答，并在回答里引用该资料；系统会把它标为用户材料，不会当成三位作者的原文证据
- 学习笔记：可以手写笔记，也可以把某次回答保存为笔记；笔记不会自动变成作者证据，只有手动转成“我的资料”后才参与问答
- 连续追问：会话会保留上下文，用于第二轮、第三轮继续问
- 换题不串题：完整新问题会重新检索；例如主图问题后再问人群画像、选品实操，不会继续沿用上一题主图结论
- 来源引用：回答会附带作者、文章标题、摘录和原文线索
- 本轮运行成本：页面展示本机运行的云端 token 为 0，并给出如果改接云模型时的大致 token 参考
- 问前资料选择和意图确认：输入问题后可以先判断本轮该限定哪些来源，并确认是方法学习、产品诊断还是实验复盘，再用这些资料提问
- 本轮结果确认：每次回答后可以标记“这次有效、需要补来源、切换意图、补产品数据”，确认结果会留在本地历史里，并把下一步追问填好
- 学习闭环状态：左侧会汇总本会话回答数、有效数、待确认数和待处理数，并根据用户确认结果给出下一步动作
- 反馈驱动下一轮检索：如果用户标记“需要补来源、切换意图、补产品数据、这次有效”，下一轮会把这个确认作为检索背景；当前问题和历史反馈里的产品数据只保留通用主题，不会混成作者原文证据
- 本轮图谱：每次回答下方展示问题、要点、概念、来源和作者之间的关系
- 知识缺口雷达：每次回答会提示下一步优先补的作者来源、产品数据或复核动作；它只做学习导航，不会改变作者原文证据边界
- 来源决策表：每次回答把作者原文拆成“能支持什么、不能证明什么、下一步要补什么数据”，每行都能回到原文上下文，并可导出 Markdown 或 CSV 作为复核清单
- 下一步资料选择：每次回答会推荐先读哪条来源、还要补哪类材料，并明确推荐理由不是新的作者证据
- 有限批次深加工：来源树增强只能在页面按 10/50/250 条分批启动，每批跑完自动暂停，避免长时间无边界运行
- 本地学习包预览：专题会话可阅读文字报告、复习卡、思维导图预览、来源表、掌握度自测和亚马逊行动实验计划，并可导出 Markdown、JSON、复习卡 CSV、来源表 CSV；完整来源树学习仍以状态栏为准

## 明确不包含

- 本次只交付文字问答、来源阅读、学习路径和本地知识库验证，不包含音频或视频录制、转写、播放、剪辑、生成或上传功能

## 验收方式

- 打开 ${report.url}，提问“主图视觉点击率转化率怎么优化？”
- 输入问题后先点“先选资料”，确认出现“问前资料选择”和意图选项，再点“用这些资料提问”
- 继续追问“那我应该先改哪一块？”
- 在回答下方点击“本轮结果确认”里的“需要补来源”或“补产品数据”，确认输入框出现下一步追问
- 检查左侧“学习闭环状态”，确认它会随着“这次有效、需要补来源、切换意图、补产品数据”的点击同步变化
- 标记“需要补来源”后继续提问，确认下一轮仍能返回来源、图谱和知识缺口雷达
- 在同一会话连续问“人群画像应该怎么构建？”和“列出所有选品实操的可落地执行方法？”，确认新主题不会沿用上一题主图结论
- 检查每条回答下方是否出现“知识缺口雷达”，点击“继续追问”后应把下一步问题填入输入框
- 检查每条回答下方是否出现“下一步资料选择”，点击“定位来源”后应跳到对应来源卡片
- 点击某条回答下方“保存回答为笔记”，确认左侧“学习笔记”出现记录，再把它手动转成“我的资料”
- 在“我的资料”里粘贴一段带唯一测试词的资料，勾选“本轮只问已勾选的我的资料”，确认回答只引用这份资料
- 在左侧专题会话里点击“学习包”，检查是否出现本地学习包预览、复习卡预览、思维导图预览、掌握度自测、亚马逊行动实验计划和来源数据表
- 运行 \`${PRODUCT_COMMAND} acceptance\`，应返回三类真实问题、主图追问、换题不串题、来源账本、复习卡、思维导图节点、掌握度自测和行动实验计划数量

## 下一步

${report.nextActions.map((item) => `- ${item}`).join("\n")}
`;
}

function printHumanReport(report) {
  console.log(`入口：${report.url}`);
  console.log(`总体：${report.level}`);
  console.log(`问答：${report.service.answerStatus === "ready" ? "可问答、可引用" : "未就绪"}`);
  console.log(`资料：${report.service.documents} 篇，我的资料 ${report.service.userSourceCount || 0} 份，学习笔记 ${report.service.learningNoteCount || 0} 条，片段 ${report.service.chunks} 个，语义索引 ${report.service.embedded}/${report.service.chunks}，覆盖率 ${report.service.coverage}%。`);
  console.log(`来源树：等待 ${report.service.sourceTreeQueuedJobs}，完成 ${report.service.sourceTreeDoneJobs}，失败 ${report.service.sourceTreeFailedJobs}。`);
  if (report.service.sourceTreeEstimatedRemainingText) {
    console.log(`来源树预计：按最近速度约 ${report.service.sourceTreeEstimatedRemainingText}；不影响当前问答和引用。`);
  }
  console.log(`本地模型：${report.localAi.preflight.ok ? "就绪" : "未就绪"}。${report.localAi.preflight.message}`);
  if (report.critical.length) {
    console.log("\n必须处理：");
    report.critical.forEach((item) => console.log(`- ${item}`));
  }
  if (report.warnings.length) {
    console.log("\n提醒：");
    report.warnings.forEach((item) => console.log(`- ${item}`));
  }
  if (report.nextActions.length) {
    console.log("\n下一步：");
    report.nextActions.forEach((item) => console.log(`- ${item}`));
  }
}

function shellQuote(value) {
  return `'${String(value).replace(/'/g, "'\\''")}'`;
}

function formatMinutes(value) {
  const minutes = Math.max(0, Number(value || 0));
  if (!Number.isFinite(minutes) || minutes <= 0) return "";
  if (minutes < 60) return `${Math.ceil(minutes)} 分钟`;
  const hours = minutes / 60;
  if (hours < 48) return `${Math.ceil(hours)} 小时`;
  return `${Math.ceil(hours / 24)} 天`;
}

function unique(items) {
  return [...new Set(items.map((item) => String(item || "").trim()).filter(Boolean))];
}

if (import.meta.url === pathToFileURL(process.argv[1] || "").href) {
  main().catch((error) => {
    console.error(error?.stack || error?.message || String(error));
    process.exit(1);
  });
}
