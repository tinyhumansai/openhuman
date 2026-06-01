import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { buildCompletionAudit, buildProductDoctorReport, completionAuditMarkdown, handoffMarkdown } from "./amazon-qa-product.mjs";

const READY_STATUS = {
  health: {
    documents: 1779,
    chunks: 14597,
    embeddedChunks: 14597,
    vectorCoveragePercent: 100,
  },
  readiness: {
    level: "answer_ready_learning_processing",
    answerStatus: "ready",
    searchStatus: "ready",
    citationStatus: "ready",
    learningStatus: "processing",
  },
  sourceTreeDrain: {
    state: "paused",
    message: "来源树深加工本轮批次已暂停，本轮处理 10 个任务，剩余约 10 个。",
    processedJobs: 10,
    queuedJobs: 10,
    readyJobs: 10,
    runningJobs: 0,
    doneJobs: 20,
    failedJobs: 0,
    jobsPerMinute: 1.5,
    estimatedMinutesRemaining: 90,
  },
};

test("product doctor marks semantic Q&A ready while keeping Vercel boundary explicit", async () => {
  const restore = mockFetch({
    status: READY_STATUS,
    models: ["mxbai-embed-large:latest", "qwen2.5:3b"],
  });
  try {
    const report = await buildProductDoctorReport({
      baseUrl: "http://127.0.0.1:7790",
      ollamaEndpoint: "http://127.0.0.1:11434",
    });

    assert.equal(report.ok, true);
    assert.equal(report.service.answerStatus, "ready");
    assert.equal(report.service.embedded, 14597);
    assert.equal(report.service.learningNoteCount, 0);
    assert.equal(report.service.sourceTreeDrainState, "paused");
    assert.equal(report.service.sourceTreeJobsPerMinute, 1.5);
    assert.equal(report.service.sourceTreeEstimatedMinutesRemaining, 90);
    assert.equal(report.service.sourceTreeEstimatedRemainingText, "2 小时");
    assert.equal(report.localAi.preflight.ok, true);
    assert.equal(report.deployment.vercelReady, false);
    assert.match(report.deployment.reason, /本地 SQLite/);
    assert.ok(report.warnings.some((item) => item.includes("来源树仍在后台深加工")));
  } finally {
    restore();
  }
});

test("product doctor fails the delivery gate when semantic vectors are incomplete", async () => {
  const restore = mockFetch({
    status: {
      ...READY_STATUS,
      health: { ...READY_STATUS.health, embeddedChunks: 100, vectorCoveragePercent: 0.7 },
      readiness: { ...READY_STATUS.readiness, answerStatus: "partial", searchStatus: "partial" },
    },
    models: ["mxbai-embed-large:latest", "qwen2.5:3b"],
  });
  try {
    const report = await buildProductDoctorReport({
      baseUrl: "http://127.0.0.1:7790",
      ollamaEndpoint: "http://127.0.0.1:11434",
    });

    assert.equal(report.ok, false);
    assert.ok(report.critical.some((item) => item.includes("语义索引未满")));
    assert.ok(report.critical.some((item) => item.includes("问答和引用未达到就绪")));
  } finally {
    restore();
  }
});

test("handoff uses real status, local deployment boundary, and no audio/video scope", async () => {
  const report = {
    ok: false,
    warnings: [],
    generatedAt: "2026-05-31T00:00:00.000Z",
    url: "http://127.0.0.1:7790",
    service: {
      answerStatus: "partial",
      documents: 100,
      userSourceCount: 1,
      learningNoteCount: 2,
      chunks: 200,
      embedded: 20,
      coverage: 10,
      learningStatus: "processing",
      sourceTreeQueuedJobs: 9,
      sourceTreeDoneJobs: 1,
      sourceTreeFailedJobs: 0,
      sourceTreeJobsPerMinute: 1.5,
      sourceTreeEstimatedRemainingText: "2 小时",
    },
    deployment: {
      reason: "本地依赖",
      realisticTarget: "本机或 VPS",
    },
    nextActions: ["补齐语义索引"],
  };
  const markdown = handoffMarkdown(report);

  assert.match(markdown, /交付边界先说明/);
  assert.match(markdown, /未通过本机验收/);
  assert.match(markdown, /本机入口/);
  assert.match(markdown, /不是 Vercel 线上部署/);
  assert.match(markdown, /来源树预计：按最近速度约 2 小时/);
  assert.match(markdown, /100 篇资料、200 个片段，语义索引 20\/200/);
  assert.match(markdown, /来源树速度：1.5 个\/分钟；预计剩余 2 小时/);
  assert.match(markdown, /本地模型主回答/);
  assert.match(markdown, /换题不串题/);
  assert.match(markdown, /人群画像、选品实操/);
  assert.match(markdown, /acceptance/);
  assert.match(await readFile(new URL("./amazon-qa-product.mjs", import.meta.url), "utf8"), /终版真实问答、追问和换题不串题验收/);
  assert.match(markdown, /来源决策表[\s\S]*导出 Markdown 或 CSV/);
  assert.match(markdown, /本地学习包预览/);
  assert.match(markdown, /不包含音频或视频/);
  assert.match(await readFile(new URL("./amazon-qa-product.mjs", import.meta.url), "utf8"), /command === "acceptance"/);
  assert.doesNotMatch(markdown, /1779 篇资料、14597 个片段已完成/);
  assert.doesNotMatch(markdown, /## 已交付能力/);
});

test("Vercel entry page is a static delivery boundary, not a cloud Q&A shell", async () => {
  const html = await readFile(new URL("./vercel-entry/index.html", import.meta.url), "utf8");
  const vercelConfig = await readFile(new URL("./vercel-entry/vercel.json", import.meta.url), "utf8");

  assert.match(html, /亚马逊学习问答本地产品入口/);
  assert.match(html, /http:\/\/127\.0\.0\.1:7790/);
  assert.match(html, /1779/);
  assert.match(html, /14597/);
  assert.match(html, /100%/);
  assert.match(html, /9622/);
  assert.match(html, /不是云端完整问答服务/);
  assert.match(html, /不能原样在 Vercel Serverless 环境中运行/);
  assert.match(html, /不包含音频或视频功能/);
  assert.doesNotMatch(html, /\/api\/ask/);
  assert.doesNotMatch(html, /textarea/);
  assert.doesNotThrow(() => JSON.parse(vercelConfig));
});

test("completion audit refuses to mark the goal complete while source tree and cloud Q&A are incomplete", () => {
  const report = {
    ok: true,
    generatedAt: "2026-06-01T00:00:00.000Z",
    service: {
      answerStatus: "ready",
      learningStatus: "processing",
      documents: 1779,
      chunks: 14597,
      embedded: 14597,
      coverage: 100,
      sourceTreeQueuedJobs: 9622,
      sourceTreeDoneJobs: 2918,
      sourceTreeFailedJobs: 0,
      sourceTreeEstimatedRemainingText: "5 天",
    },
    deployment: {
      vercelReady: false,
    },
    acceptanceEvidence: null,
  };
  const audit = buildCompletionAudit(report);
  const markdown = completionAuditMarkdown(audit);

  assert.equal(audit.canMarkGoalComplete, false);
  assert.equal(audit.completionStatus, "local_qa_ready_not_full_final");
  assert.ok(audit.requirements.some((item) => item.id === "local_semantic_knowledge_base" && item.status === "proved"));
  assert.ok(audit.requirements.some((item) => item.id === "interactive_memory_qa" && item.status === "needs_acceptance_evidence"));
  assert.ok(audit.requirements.some((item) => item.id === "source_tree_learning_layer" && item.status === "not_complete"));
  assert.ok(audit.requirements.some((item) => item.id === "vercel_delivery" && item.status === "boundary_only"));
  assert.ok(audit.requirements.some((item) => item.id === "no_audio_video" && item.status === "proved"));
  assert.match(markdown, /尚未达到完整终版/);
  assert.match(markdown, /来源树仍有 9622 个后台任务/);
  assert.match(markdown, /不能原样部署到 Vercel Serverless/);
});

test("completion audit accepts saved real-question evidence for interactive Q&A", () => {
  const report = {
    ok: true,
    generatedAt: "2026-06-01T00:00:00.000Z",
    service: {
      answerStatus: "ready",
      learningStatus: "processing",
      documents: 1779,
      chunks: 14597,
      embedded: 14597,
      coverage: 100,
      sourceTreeQueuedJobs: 9622,
      sourceTreeDoneJobs: 2918,
      sourceTreeFailedJobs: 0,
      sourceTreeEstimatedRemainingText: "5 天",
    },
    deployment: { vercelReady: false },
    acceptanceEvidence: {
      generatedAt: "2026-06-01T00:10:00.000Z",
      result: acceptanceEvidenceResult(),
    },
  };
  const audit = buildCompletionAudit(report);
  const interactive = audit.requirements.find((item) => item.id === "interactive_memory_qa");

  assert.equal(interactive.status, "proved");
  assert.equal(audit.acceptanceEvidence.ok, true);
  assert.deepEqual(audit.needsAcceptance, []);
  assert.doesNotMatch(audit.summary, /真实问题验收证据/);
  assert.match(audit.summary, /来源树完成、云端完整部署条件/);
  assert.ok(audit.blocking.includes("source_tree_learning_layer"));
  assert.ok(audit.boundaryOnly.includes("vercel_delivery"));
});

function acceptanceEvidenceResult() {
  const scenario = (id) => ({
    id,
    sources: 5,
    graphNodes: 20,
    learningQueueItems: 6,
  });
  return {
    ok: true,
    documents: 1779,
    chunks: 14597,
    embeddedChunks: 14597,
    vectorCoveragePercent: 100,
    scenarios: [scenario("visual-conversion"), scenario("product-selection"), scenario("listing-keywords")],
    topicSwitch: {
      standaloneResults: [
        { id: "product-title", sources: 5, graphNodes: 20 },
        { id: "listing-prep", sources: 5, graphNodes: 21 },
        { id: "persona", sources: 4, graphNodes: 8 },
        { id: "selection-methods", sources: 5, graphNodes: 21 },
      ],
    },
    confirmationLoop: {
      status: "needs_source",
      followUpSources: 5,
    },
    studyPackSources: 7,
    studioFlashcards: 10,
    studioMindMapNodes: 22,
  };
}

function mockFetch({ status, models }) {
  const original = globalThis.fetch;
  globalThis.fetch = async (url) => {
    const value = String(url);
    if (value.endsWith("/api/status")) return jsonResponse(status);
    if (value.endsWith("/api/tags")) {
      return jsonResponse({ models: models.map((name) => ({ name })) });
    }
    throw new Error(`Unexpected fetch: ${value}`);
  };
  return () => {
    globalThis.fetch = original;
  };
}

function jsonResponse(payload) {
  return {
    ok: true,
    status: 200,
    statusText: "OK",
    async json() {
      return payload;
    },
  };
}
