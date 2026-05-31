import assert from "node:assert/strict";
import test from "node:test";

import {
  amazonSourceId,
  buildSourceTreeCalibration,
  buildMemoryTreeIngestRequest,
  buildSourceTreeDrainPreflight,
  normalizeSourceTreeDrainRun,
  sourceTreeImportPlan,
  sourceTreeSearchTerms,
  summarizeSourceTreeDrain,
  summarizeSourceTreeStatus,
} from "./amazon-source-tree-lib.mjs";

const sampleRow = {
  author: "跨境电商长期主义",
  date: "2022-06-16",
  title: "你是如何解决转化率的？",
  source_path: "跨境电商长期主义html/example.html",
  source_url: "https://mp.weixin.qq.com/s/example",
  markdown_path: "openhuman-kb/processed/articles/跨境电商长期主义/2022-06-16_你是如何解决转化率的？.md",
  chars: 1200,
};

test("amazonSourceId uses the original source path as the stable OpenHuman source id", () => {
  assert.equal(amazonSourceId(sampleRow), "跨境电商长期主义html/example.html");
});

test("buildMemoryTreeIngestRequest preserves article provenance for memory_tree document ingest", () => {
  const request = buildMemoryTreeIngestRequest(sampleRow, "# 你是如何解决转化率的？\n\n产品首图决定点击率。", {
    namespace: "amazon-learning",
  });

  assert.equal(request.source_kind, "document");
  assert.equal(request.source_id, "跨境电商长期主义html/example.html");
  assert.equal(request.owner, "跨境电商长期主义");
  assert.deepEqual(request.tags, ["amazon", "亚马逊学习", "amazon-learning", "跨境电商长期主义"]);
  assert.equal(request.payload.provider, "amazon-author-article");
  assert.equal(request.payload.title, "你是如何解决转化率的？");
  assert.match(request.payload.body, /产品首图决定点击率/);
  assert.equal(request.payload.modified_at, Date.parse("2022-06-16T00:00:00.000Z"));
  assert.equal(request.payload.source_ref, "https://mp.weixin.qq.com/s/example");
});

test("sourceTreeSearchTerms expands visual conversion questions for source-tree routing", () => {
  const terms = sourceTreeSearchTerms("主图视觉点击率转化率怎么优化？");

  assert.ok(terms.includes("主图"));
  assert.ok(terms.includes("点击率"));
  assert.ok(terms.includes("转化率"));
  assert.ok(terms.includes("Listing"));
  assert.ok(terms.includes("页面"));
});

test("buildSourceTreeCalibration keeps summary hits as routing hints, not evidence", () => {
  const calibration = buildSourceTreeCalibration({
    query: "主图视觉点击率转化率怎么优化？",
    terms: ["主图", "点击率", "转化率"],
    chunkRows: [
      {
        sourceId: "跨境电商长期主义html/example.html",
        sourceRef: "https://mp.weixin.qq.com/s/example",
        owner: "跨境电商长期主义",
        chunkCount: 2,
      },
    ],
    summaryRows: [
      {
        id: "summary-1",
        treeId: "tree-1",
        content: "这是一段来源树摘要，提示主图和转化率相关。",
      },
    ],
    resolvedSources: [
      {
        sourceId: "跨境电商长期主义html/example.html",
        sourceRef: "https://mp.weixin.qq.com/s/example",
        owner: "跨境电商长期主义",
        title: "你是如何解决转化率的？",
      },
    ],
  });

  assert.equal(calibration.status, "active");
  assert.equal(calibration.candidateCount, 1);
  assert.equal(calibration.resolvedSourceCount, 1);
  assert.equal(calibration.summaryHintCount, 1);
  assert.equal(calibration.candidates[0].type, "route_hint");
  assert.equal(calibration.candidates[0].matchedOriginalSource, true);
  assert.equal(calibration.summaries[0].type, "summary_hint");
  assert.equal(calibration.summaries[0].canUseAsEvidence, false);
  assert.match(calibration.boundary, /原文/);
  assert.match(calibration.boundary, /不能当作者原文证据/);
});

test("buildSourceTreeCalibration still explains the boundary when no source-tree route is found", () => {
  const calibration = buildSourceTreeCalibration({
    query: "完全无关的问题",
    terms: [],
    chunkRows: [],
    summaryRows: [],
    resolvedSources: [],
  });

  assert.equal(calibration.status, "empty");
  assert.equal(calibration.candidateCount, 0);
  assert.equal(calibration.resolvedSourceCount, 0);
  assert.match(calibration.summary, /本轮没有用到 OpenHuman 来源树/);
  assert.match(calibration.boundary, /摘要/);
});

test("summarizeSourceTreeStatus keeps vector index and OpenHuman source tree coverage separate", () => {
  const summary = summarizeSourceTreeStatus({
    manifestRows: [sampleRow, { ...sampleRow, source_path: "张子卿html/another.html", author: "张子卿" }],
    stats: {
      chunks: 20,
      chunkSourceIds: ["跨境电商长期主义html/example.html"],
      ingestedSourceIds: ["跨境电商长期主义html/example.html"],
      trees: 1,
      summaries: 0,
      readyJobs: 0,
    },
  });

  assert.equal(summary.manifestDocuments, 2);
  assert.equal(summary.ingestedDocuments, 1);
  assert.equal(summary.chunkSourceDocuments, 1);
  assert.equal(summary.coveragePercent, 50);
  assert.equal(summary.level, "partial");
  assert.match(summary.message, /OpenHuman 来源树/);
  assert.match(summary.message, /1\/2/);
});

test("summarizeSourceTreeStatus distinguishes ingested leaves from generated source trees", () => {
  const summary = summarizeSourceTreeStatus({
    manifestRows: [sampleRow],
    stats: {
      chunks: 1,
      chunkSourceIds: ["跨境电商长期主义html/example.html"],
      ingestedSourceIds: ["跨境电商长期主义html/example.html"],
      trees: 0,
      summaries: 0,
      readyJobs: 2,
      runningJobs: 1,
    },
  });

  assert.equal(summary.level, "pending_tree");
  assert.equal(summary.coveragePercent, 100);
  assert.equal(summary.queuedJobs, 3);
  assert.match(summary.message, /后台还没有生成树/);
});

test("summarizeSourceTreeStatus reports background processing even after all originals are ingested", () => {
  const summary = summarizeSourceTreeStatus({
    manifestRows: [sampleRow],
    stats: {
      chunks: 3,
      chunkSourceIds: ["跨境电商长期主义html/example.html"],
      ingestedSourceIds: ["跨境电商长期主义html/example.html"],
      trees: 1,
      summaries: 0,
      readyJobs: 7,
    },
  });

  assert.equal(summary.level, "processing");
  assert.equal(summary.coveragePercent, 100);
  assert.match(summary.message, /后台还有 7 个任务待处理/);
});

test("summarizeSourceTreeStatus gives failed source-tree jobs priority over normal processing", () => {
  const summary = summarizeSourceTreeStatus({
    manifestRows: [sampleRow],
    stats: {
      chunks: 3,
      chunkSourceIds: ["跨境电商长期主义html/example.html"],
      ingestedSourceIds: ["跨境电商长期主义html/example.html"],
      trees: 1,
      summaries: 0,
      readyJobs: 7,
      failedJobs: 2,
      doneJobs: 11,
    },
  });

  assert.equal(summary.level, "needs_attention");
  assert.equal(summary.queuedJobs, 7);
  assert.equal(summary.doneJobs, 11);
  assert.match(summary.message, /2 个后台任务失败/);
  assert.match(summary.message, /仍有 7 个任务待处理/);
});

test("summarizeSourceTreeStatus gives failed jobs priority even before trees exist", () => {
  const summary = summarizeSourceTreeStatus({
    manifestRows: [sampleRow],
    stats: {
      chunks: 1,
      chunkSourceIds: ["跨境电商长期主义html/example.html"],
      ingestedSourceIds: ["跨境电商长期主义html/example.html"],
      trees: 0,
      summaries: 0,
      readyJobs: 2,
      failedJobs: 1,
    },
  });

  assert.equal(summary.level, "needs_attention");
  assert.match(summary.message, /1 个后台任务失败/);
});

test("summarizeSourceTreeStatus reports an empty OpenHuman source tree without pretending vectors are enough", () => {
  const summary = summarizeSourceTreeStatus({
    manifestRows: [sampleRow],
    stats: {
      chunks: 0,
      chunkSourceIds: [],
      ingestedSourceIds: [],
      trees: 0,
      summaries: 0,
    },
  });

  assert.equal(summary.level, "empty");
  assert.equal(summary.coveragePercent, 0);
  assert.match(summary.message, /还没有进入 OpenHuman 来源树/);
});

test("sourceTreeImportPlan excludes already ingested source ids and obeys a limit", () => {
  const rows = [
    sampleRow,
    { ...sampleRow, source_path: "张子卿html/another.html", author: "张子卿" },
    { ...sampleRow, source_path: "飞翔的波波html/third.html", author: "飞翔的波波" },
  ];
  const plan = sourceTreeImportPlan(rows, {
    alreadyIngestedSourceIds: ["跨境电商长期主义html/example.html"],
    limit: 1,
  });

  assert.equal(plan.total, 3);
  assert.equal(plan.alreadyIngested, 1);
  assert.equal(plan.toImport.length, 1);
  assert.equal(plan.toImport[0].source_path, "张子卿html/another.html");
});

test("buildSourceTreeDrainPreflight blocks drain when Ollama is unreachable", () => {
  const preflight = buildSourceTreeDrainPreflight({
    ok: false,
    endpoint: "http://127.0.0.1:11434",
    requiredModels: ["mxbai-embed-large:latest", "qwen2.5:3b"],
    error: "connection refused",
  });

  assert.equal(preflight.ok, false);
  assert.equal(preflight.level, "local_ai_unavailable");
  assert.match(preflight.message, /Ollama/);
  assert.match(preflight.message, /connection refused/);
});

test("buildSourceTreeDrainPreflight blocks drain when required local models are missing", () => {
  const preflight = buildSourceTreeDrainPreflight({
    ok: true,
    endpoint: "http://127.0.0.1:11434",
    availableModels: ["mxbai-embed-large:latest"],
    requiredModels: ["mxbai-embed-large:latest", "qwen2.5:3b"],
  });

  assert.equal(preflight.ok, false);
  assert.equal(preflight.level, "missing_models");
  assert.deepEqual(preflight.missingModels, ["qwen2.5:3b"]);
  assert.match(preflight.message, /qwen2.5:3b/);
});

test("buildSourceTreeDrainPreflight passes when Ollama exposes all required models", () => {
  const preflight = buildSourceTreeDrainPreflight({
    ok: true,
    endpoint: "http://127.0.0.1:11434",
    availableModels: ["mxbai-embed-large:latest", "qwen2.5:3b"],
    requiredModels: ["mxbai-embed-large:latest", "qwen2.5:3b"],
  });

  assert.equal(preflight.ok, true);
  assert.equal(preflight.level, "ready");
  assert.deepEqual(preflight.missingModels, []);
});

test("summarizeSourceTreeDrain shows a runnable queued source-tree processor", () => {
  const summary = summarizeSourceTreeDrain({
    sourceTree: { queuedJobs: 10386, failedJobs: 0, ingestedDocuments: 1779 },
    run: { state: "idle" },
  });

  assert.equal(summary.level, "idle");
  assert.equal(summary.canStart, true);
  assert.equal(summary.running, false);
  assert.match(summary.message, /未运行/);
  assert.match(summary.message, /10,386/);
});

test("summarizeSourceTreeDrain surfaces local AI preflight failures", () => {
  const summary = summarizeSourceTreeDrain({
    sourceTree: { queuedJobs: 100, failedJobs: 0, ingestedDocuments: 1779 },
    run: {
      state: "preflight_failed",
      error: "Ollama 不可用，无法启动来源树深加工。",
      updatedAt: "2026-05-31T09:00:00.000Z",
    },
  });

  assert.equal(summary.level, "needs_attention");
  assert.equal(summary.canStart, true);
  assert.match(summary.message, /Ollama 不可用/);
});

test("summarizeSourceTreeDrain keeps running batches visible and not startable twice", () => {
  const now = Date.parse("2026-05-26T00:00:00.000Z");
  const summary = summarizeSourceTreeDrain({
    now,
    sourceTree: { queuedJobs: 100, failedJobs: 0, ingestedDocuments: 1779 },
    run: {
      state: "running",
      pid: 1234,
      runId: "run-1",
      logPath: "/tmp/source-tree-drain-run-1.log",
      processedJobs: 25,
      startedAt: "2026-05-25T23:59:00.000Z",
      updatedAt: "2026-05-25T23:59:59.000Z",
    },
  });

  assert.equal(summary.level, "running");
  assert.equal(summary.running, true);
  assert.equal(summary.canStart, false);
  assert.equal(summary.processedJobs, 25);
  assert.equal(summary.runId, "run-1");
  assert.equal(summary.elapsedSeconds, 60);
  assert.equal(summary.jobsPerMinute, 25);
  assert.equal(summary.estimatedMinutesRemaining, 4);
  assert.equal(summary.readyJobs, 0);
  assert.equal(summary.runningJobs, 0);
  assert.equal(summary.doneJobs, 0);
  assert.match(summary.logPath, /source-tree-drain-run-1/);
  assert.match(summary.message, /正在运行/);
  assert.match(summary.message, /约 4 分钟/);
});

test("summarizeSourceTreeDrain exposes queue diagnostics from the latest batch", () => {
  const now = Date.parse("2026-05-26T00:00:00.000Z");
  const summary = summarizeSourceTreeDrain({
    now,
    sourceTree: { queuedJobs: 74, readyJobs: 73, runningJobs: 1, doneJobs: 26, failedJobs: 0, ingestedDocuments: 1779 },
    run: {
      state: "running",
      pid: 1234,
      processedJobs: 25,
      readyJobs: 73,
      runningJobs: 1,
      doneJobs: 26,
      startedAt: "2026-05-25T23:59:00.000Z",
      updatedAt: "2026-05-25T23:59:59.000Z",
      lastBatch: {
        processed: 25,
        limit: 25,
        configuredBatchSize: 25,
        beforeQueuedJobs: 100,
        afterQueuedJobs: 74,
        queuedDelta: 26,
        beforeDoneJobs: 0,
        afterDoneJobs: 26,
        doneDelta: 26,
      },
    },
  });

  assert.equal(summary.readyJobs, 73);
  assert.equal(summary.runningJobs, 1);
  assert.equal(summary.doneJobs, 26);
  assert.equal(summary.lastBatch.processed, 25);
  assert.equal(summary.lastBatch.queuedDelta, 26);
  assert.equal(summary.lastBatch.doneDelta, 26);
});

test("summarizeSourceTreeDrain prefers live source-tree counts over stale run snapshots", () => {
  const summary = summarizeSourceTreeDrain({
    sourceTree: { queuedJobs: 96, readyJobs: 95, runningJobs: 1, doneJobs: 4, failedJobs: 0, ingestedDocuments: 1779 },
    run: {
      state: "running",
      processAlive: true,
      processedJobs: 0,
      readyJobs: 100,
      runningJobs: 0,
      doneJobs: 0,
      updatedAt: new Date().toISOString(),
    },
  });

  assert.equal(summary.queuedJobs, 96);
  assert.equal(summary.readyJobs, 95);
  assert.equal(summary.runningJobs, 1);
  assert.equal(summary.doneJobs, 4);
});

test("summarizeSourceTreeDrain freezes paused run timing at finish time", () => {
  const summary = summarizeSourceTreeDrain({
    now: Date.parse("2026-05-26T01:00:00.000Z"),
    sourceTree: { queuedJobs: 96, failedJobs: 0, ingestedDocuments: 1779 },
    run: {
      state: "paused",
      processedJobs: 3,
      startedAt: "2026-05-26T00:00:00.000Z",
      updatedAt: "2026-05-26T00:01:00.000Z",
      finishedAt: "2026-05-26T00:01:00.000Z",
    },
  });

  assert.equal(summary.level, "paused");
  assert.equal(summary.elapsedSeconds, 60);
  assert.equal(summary.jobsPerMinute, 3);
  assert.equal(summary.estimatedMinutesRemaining, 32);
});

test("summarizeSourceTreeDrain still reports elapsed time before first job completes", () => {
  const now = Date.parse("2026-05-26T00:01:00.000Z");
  const summary = summarizeSourceTreeDrain({
    now,
    sourceTree: { queuedJobs: 100, failedJobs: 0, ingestedDocuments: 1779 },
    run: {
      state: "running",
      pid: 1234,
      runId: "run-early",
      logPath: "/tmp/source-tree-drain-run-early.log",
      processedJobs: 0,
      startedAt: "2026-05-26T00:00:00.000Z",
      updatedAt: "2026-05-26T00:00:30.000Z",
    },
  });

  assert.equal(summary.level, "running");
  assert.equal(summary.elapsedSeconds, 60);
  assert.equal(summary.jobsPerMinute, 0);
  assert.equal(summary.estimatedMinutesRemaining, 0);
});

test("normalizeSourceTreeDrainRun marks stale running state when heartbeat stops", () => {
  const normalized = normalizeSourceTreeDrainRun(
    {
      state: "running",
      processedJobs: 30,
      updatedAt: "2026-05-25T23:00:00.000Z",
    },
    Date.parse("2026-05-26T00:00:00.000Z"),
  );

  assert.equal(normalized.state, "stale");
  assert.equal(normalized.processedJobs, 30);
  assert.equal(normalized.readyJobs, 0);
  assert.equal(normalized.runningJobs, 0);
  assert.equal(normalized.doneJobs, 0);
});

test("normalizeSourceTreeDrainRun keeps old heartbeat running when the process is alive", () => {
  const normalized = normalizeSourceTreeDrainRun(
    {
      state: "running",
      processedJobs: 30,
      processAlive: true,
      updatedAt: "2026-05-25T23:00:00.000Z",
    },
    Date.parse("2026-05-26T00:00:00.000Z"),
  );

  assert.equal(normalized.state, "running");
  assert.equal(normalized.processedJobs, 30);
});

test("summarizeSourceTreeDrain treats stop requests as active but not startable", () => {
  const now = Date.parse("2026-05-26T00:00:00.000Z");
  const summary = summarizeSourceTreeDrain({
    now,
    sourceTree: { queuedJobs: 99, failedJobs: 0, ingestedDocuments: 1779 },
    run: {
      state: "stopping",
      processedJobs: 12,
      stopRequested: true,
      updatedAt: "2026-05-25T23:59:59.000Z",
    },
  });

  assert.equal(summary.level, "stopping");
  assert.equal(summary.running, true);
  assert.equal(summary.canStart, false);
  assert.equal(summary.canStop, true);
  assert.equal(summary.stopRequested, true);
  assert.match(summary.message, /正在暂停/);
});
