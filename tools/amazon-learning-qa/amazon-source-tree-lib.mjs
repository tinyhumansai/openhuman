export function amazonSourceId(row = {}) {
  return firstText(row.source_path, row.sourcePath, row.source_url, row.sourceUrl, row.markdown_path, row.title);
}

export function buildMemoryTreeIngestRequest(row = {}, markdown = "", options = {}) {
  const namespace = firstText(options.namespace, "amazon-learning");
  const sourceId = amazonSourceId(row);
  if (!sourceId) throw new Error("Cannot build source-tree ingest request without a stable source id.");

  const author = firstText(row.author, "unknown-author");
  const dateMs = articleDateToMs(row.date);
  const sourceRef = firstText(row.source_url, row.sourceUrl, row.source_path, row.sourcePath);

  return {
    source_kind: "document",
    source_id: sourceId,
    owner: author,
    tags: uniqueTexts(["amazon", "亚马逊学习", namespace, author]),
    payload: {
      provider: "amazon-author-article",
      title: firstText(row.title, sourceId),
      body: String(markdown || ""),
      modified_at: dateMs,
      source_ref: sourceRef,
    },
  };
}

export function summarizeSourceTreeStatus({ manifestRows = [], stats = {} } = {}) {
  const manifestSourceIds = uniqueTexts(manifestRows.map(amazonSourceId));
  const ingestedIds = intersectStableIds(stats.ingestedSourceIds, manifestSourceIds);
  const chunkSourceIds = intersectStableIds(stats.chunkSourceIds, manifestSourceIds);
  const manifestDocuments = manifestSourceIds.length;
  const ingestedDocuments = ingestedIds.length;
  const chunkSourceDocuments = chunkSourceIds.length;
  const chunks = Math.max(0, Number(stats.chunks || 0));
  const trees = Math.max(0, Number(stats.trees || 0));
  const summaries = Math.max(0, Number(stats.summaries || 0));
  const readyJobs = Math.max(0, Number(stats.readyJobs || 0));
  const runningJobs = Math.max(0, Number(stats.runningJobs || 0));
  const failedJobs = Math.max(0, Number(stats.failedJobs || 0));
  const doneJobs = Math.max(0, Number(stats.doneJobs || 0));
  const queuedJobs = readyJobs + runningJobs;
  const coveragePercent = manifestDocuments > 0 ? Math.round((ingestedDocuments / manifestDocuments) * 1000) / 10 : 0;
  const authors = uniqueTexts(manifestRows.map((row) => row.author));
  const years = uniqueTexts(manifestRows.map((row) => String(row.date || "").slice(0, 4)).filter((year) => /^\d{4}$/.test(year))).sort();

  let level = "ok";
  let message = `OpenHuman 来源树已覆盖 ${ingestedDocuments}/${manifestDocuments} 篇作者原文，已生成 ${trees} 棵树。`;
  if (manifestDocuments > 0 && ingestedDocuments === 0) {
    level = "empty";
    message = "作者原文还没有进入 OpenHuman 来源树；当前只证明语义索引可问答，不代表来源树已建立。";
  } else if (failedJobs > 0) {
    level = "needs_attention";
    message = `OpenHuman 来源树已覆盖 ${ingestedDocuments}/${manifestDocuments} 篇作者原文，但有 ${failedJobs} 个后台任务失败，需要检查；仍有 ${queuedJobs} 个任务待处理。`;
  } else if (ingestedDocuments > 0 && trees === 0) {
    level = "pending_tree";
    message = `已有 ${ingestedDocuments}/${manifestDocuments} 篇作者原文进入 OpenHuman memory_tree，但来源树后台还没有生成树。待处理任务 ${queuedJobs} 个。`;
  } else if (queuedJobs > 0) {
    level = "processing";
    message = `OpenHuman 来源树已接收 ${ingestedDocuments}/${manifestDocuments} 篇作者原文，已生成 ${trees} 棵树，后台还有 ${queuedJobs} 个任务待处理。`;
  } else if (ingestedDocuments < manifestDocuments) {
    level = "partial";
    message = `OpenHuman 来源树只覆盖 ${ingestedDocuments}/${manifestDocuments} 篇作者原文，已生成 ${trees} 棵树，尚未完成。`;
  }

  return {
    manifestDocuments,
    ingestedDocuments,
    chunkSourceDocuments,
    chunks,
    trees,
    summaries,
    readyJobs,
    runningJobs,
    failedJobs,
    doneJobs,
    queuedJobs,
    authors: authors.length,
    years,
    coveragePercent,
    level,
    message,
  };
}

export function summarizeSourceTreeDrain({ sourceTree = {}, run = {}, now = Date.now() } = {}) {
  const normalized = normalizeSourceTreeDrainRun(run, now);
  const queuedJobs = Math.max(0, Number(sourceTree.queuedJobs || sourceTree.readyJobs || 0));
  const failedJobs = Math.max(0, Number(sourceTree.failedJobs || 0));
  const processedJobs = Math.max(0, Number(normalized.processedJobs || 0));
  const readyJobs = Math.max(0, Number(sourceTree.readyJobs ?? normalized.readyJobs ?? 0));
  const runningJobs = Math.max(0, Number(sourceTree.runningJobs ?? normalized.runningJobs ?? 0));
  const doneJobs = Math.max(0, Number(sourceTree.doneJobs ?? normalized.doneJobs ?? 0));
  const running = normalized.state === "running";
  const starting = normalized.state === "starting";
  const stopping = normalized.state === "stopping";
  const stale = normalized.state === "stale";
  const complete = queuedJobs === 0 && failedJobs === 0 && Number(sourceTree.ingestedDocuments || 0) > 0;
  const endedAtMs = Date.parse(normalized.finishedAt || normalized.updatedAt || "");
  const progressNow = running || starting || stopping || stale || !Number.isFinite(endedAtMs) ? now : endedAtMs;
  const progress = sourceTreeDrainProgress({ processedJobs, queuedJobs, startedAt: normalized.startedAt, now: progressNow });
  const lastBatch = normalizeSourceTreeLastBatch(normalized.lastBatch);
  const recommendation = sourceTreeDrainRecommendation({
    queuedJobs,
    failedJobs,
    running: running || starting || stopping,
    complete,
    jobsPerMinute: progress.jobsPerMinute,
    lastBatch,
  });

  let level = "idle";
  let message = queuedJobs > 0
    ? `来源树深加工未运行，还有 ${formatDrainNumber(queuedJobs)} 个任务等待处理。`
    : "来源树深加工当前没有等待任务。";
  if (complete) {
    level = "complete";
    message = "来源树深加工任务已处理完。";
  } else if (stopping) {
    level = "stopping";
    message = `来源树深加工正在暂停，会在当前任务结束后停下；本轮已处理 ${formatDrainNumber(processedJobs)} 个任务${progress.estimatedMinutesRemaining ? `，按当前速度约 ${formatDrainDuration(progress.estimatedMinutesRemaining)}。` : "。"}`;
  } else if (running || starting) {
    level = "running";
    message = `来源树深加工正在运行，本轮已处理 ${formatDrainNumber(processedJobs)} 个任务，剩余约 ${formatDrainNumber(queuedJobs)} 个${progress.estimatedMinutesRemaining ? `，按当前速度约 ${formatDrainDuration(progress.estimatedMinutesRemaining)}。` : "。"}`;
  } else if (stale) {
    level = "stale";
    message = `上次来源树深加工可能中断，本轮已处理 ${formatDrainNumber(processedJobs)} 个任务，剩余约 ${formatDrainNumber(queuedJobs)} 个${progress.estimatedMinutesRemaining ? `，按当前速度约 ${formatDrainDuration(progress.estimatedMinutesRemaining)}。` : "。"}`;
  } else if (normalized.state === "preflight_failed") {
    level = "needs_attention";
    message = normalized.error || "来源树深加工启动前检查未通过，请确认本地 Ollama 和所需模型可用。";
  } else if (normalized.state === "failed" || failedJobs > 0) {
    level = "needs_attention";
    message = `来源树深加工需要处理失败任务：${formatDrainNumber(failedJobs)} 个失败，${formatDrainNumber(queuedJobs)} 个等待。`;
  } else if (normalized.state === "paused") {
    level = "paused";
    message = `来源树深加工本轮批次已暂停，本轮处理 ${formatDrainNumber(processedJobs)} 个任务，剩余约 ${formatDrainNumber(queuedJobs)} 个。`;
  }

  return {
    level,
    state: normalized.state,
    running: running || starting || stopping,
    canStart: queuedJobs > 0 && !(running || starting || stopping),
    canStop: running || starting || stopping,
    stopRequested: Boolean(normalized.stopRequested || stopping),
    processedJobs,
    queuedJobs,
    readyJobs,
    runningJobs,
    failedJobs,
    doneJobs,
    elapsedSeconds: progress.elapsedSeconds,
    jobsPerMinute: progress.jobsPerMinute,
    estimatedMinutesRemaining: progress.estimatedMinutesRemaining,
    pid: normalized.pid,
    startedAt: normalized.startedAt,
    updatedAt: normalized.updatedAt,
    finishedAt: normalized.finishedAt,
    runId: normalized.runId,
    logPath: normalized.logPath,
    error: normalized.error,
    stopReason: normalized.stopReason,
    lastBatch,
    recommendation,
    activeBatch: normalized.activeBatch,
    message,
  };
}

function normalizeSourceTreeLastBatch(batch) {
  if (!batch || typeof batch !== "object") return undefined;
  const processed = Math.max(0, Number(batch.processed || 0));
  const queuedDelta = Number(batch.queuedDelta || 0);
  const doneDelta = Math.max(0, Number(batch.doneDelta || 0));
  const spawnedJobs = Math.max(0, doneDelta - queuedDelta);
  const netQueueReduction = Math.max(0, queuedDelta);
  return {
    ...batch,
    processed,
    queuedDelta,
    doneDelta,
    spawnedJobs,
    netQueueReduction,
    queueExpanded: queuedDelta < 0,
    queueHeldBySpawnedJobs: doneDelta > 0 && queuedDelta <= 0,
  };
}

function sourceTreeDrainRecommendation({ queuedJobs = 0, failedJobs = 0, running = false, complete = false, jobsPerMinute = 0, lastBatch } = {}) {
  if (complete || queuedJobs <= 0) {
    return {
      level: "complete",
      jobs: 0,
      batchSize: 0,
      sleepMs: 0,
      label: "无需继续",
      reason: "来源树深加工当前没有等待任务。",
    };
  }
  if (failedJobs > 0) {
    return {
      level: "needs_attention",
      jobs: 0,
      batchSize: 0,
      sleepMs: 0,
      label: "先查失败",
      reason: `已有 ${formatDrainNumber(failedJobs)} 个失败任务，先查看日志和失败原因，再继续跑批次。`,
    };
  }
  if (running) {
    return {
      level: "running",
      jobs: 0,
      batchSize: 0,
      sleepMs: 0,
      label: "等待当前批次",
      reason: "当前已有有限批次在运行，先等它结束或暂停。",
    };
  }
  if (lastBatch?.queueHeldBySpawnedJobs || lastBatch?.queueExpanded) {
    return {
      level: "small_batch",
      jobs: 10,
      batchSize: 2,
      sleepMs: 250,
      label: "建议先跑 10 个",
      reason: "上一批产生了后续摘要任务，等待数没有明显下降；先用小批次观察速度和失败数。",
    };
  }
  if (jobsPerMinute > 0 && jobsPerMinute < 3) {
    return {
      level: "slow",
      jobs: 10,
      batchSize: 2,
      sleepMs: 250,
      label: "建议先跑 10 个",
      reason: `当前速度约 ${formatDrainNumber(jobsPerMinute)} 个/分钟，先小批次推进，避免长时间占用本机。`,
    };
  }
  if (jobsPerMinute >= 10 && queuedJobs >= 250) {
    return {
      level: "fast",
      jobs: 250,
      batchSize: 25,
      sleepMs: 500,
      label: "可跑 250 个",
      reason: "当前速度较快且未发现失败任务，可以用较大有限批次继续推进。",
    };
  }
  return {
    level: "normal",
    jobs: Math.min(50, queuedJobs),
    batchSize: 5,
    sleepMs: 250,
    label: "建议跑 50 个",
    reason: "当前未发现失败任务，可用中等有限批次继续推进；每批结束后再观察等待数和失败数。",
  };
}

export function buildSourceTreeDrainPreflight(input = {}) {
  const endpoint = String(input.endpoint || "http://127.0.0.1:11434").replace(/\/+$/, "");
  const requiredModels = uniqueTexts(input.requiredModels || []);
  const availableModels = uniqueTexts(input.availableModels || []);
  const error = String(input.error || "").trim();
  if (input.ok !== true) {
    return {
      ok: false,
      level: "local_ai_unavailable",
      endpoint,
      requiredModels,
      availableModels,
      missingModels: requiredModels,
      message: `本地 Ollama 不可用，来源树深加工已停止启动。请先启动 Ollama，再重试。${error ? ` 原因：${error}` : ""}`,
    };
  }
  const availableSet = new Set(availableModels);
  const missingModels = requiredModels.filter((model) => !availableSet.has(model));
  if (missingModels.length > 0) {
    return {
      ok: false,
      level: "missing_models",
      endpoint,
      requiredModels,
      availableModels,
      missingModels,
      message: `本地 Ollama 缺少来源树深加工需要的模型：${missingModels.join("、")}。请先安装模型后再启动深加工。`,
    };
  }
  return {
    ok: true,
    level: "ready",
    endpoint,
    requiredModels,
    availableModels,
    missingModels: [],
    message: "本地 Ollama 和所需模型已就绪。",
  };
}

function sourceTreeDrainProgress({ processedJobs = 0, queuedJobs = 0, startedAt = "", now = Date.now() } = {}) {
  const startedAtMs = Date.parse(startedAt || "");
  if (!Number.isFinite(startedAtMs) || now <= startedAtMs) {
    return {
      elapsedSeconds: 0,
      jobsPerMinute: 0,
      estimatedMinutesRemaining: 0,
    };
  }
  const elapsedSeconds = Math.max(1, Math.round((now - startedAtMs) / 1000));
  const jobsPerMinute = processedJobs > 0 ? Math.round((processedJobs / elapsedSeconds) * 600) / 10 : 0;
  const estimatedMinutesRemaining = jobsPerMinute > 0 && queuedJobs > 0
    ? Math.max(1, Math.round(queuedJobs / jobsPerMinute))
    : 0;
  return {
    elapsedSeconds,
    jobsPerMinute,
    estimatedMinutesRemaining,
  };
}

export function normalizeSourceTreeDrainRun(run = {}, now = Date.now()) {
  const state = String(run?.state || "idle").trim() || "idle";
  const updatedAtMs = Date.parse(run?.updatedAt || "");
  const isFresh = Number.isFinite(updatedAtMs) && now - updatedAtMs < 5 * 60 * 1000;
  const activeStates = new Set(["running", "starting", "stopping"]);
  const processAlive = run?.processAlive === true;
  const normalizedState = activeStates.has(state) && !isFresh && !processAlive ? "stale" : state;
  return {
    state: normalizedState,
    pid: Number.isInteger(Number(run?.pid)) ? Number(run.pid) : null,
    runId: String(run?.runId || ""),
    logPath: String(run?.logPath || ""),
    error: String(run?.error || ""),
    stopReason: String(run?.stopReason || ""),
    processedJobs: Math.max(0, Number(run?.processedJobs || run?.processed || 0)),
    readyJobs: Math.max(0, Number(run?.readyJobs || 0)),
    runningJobs: Math.max(0, Number(run?.runningJobs || 0)),
    doneJobs: Math.max(0, Number(run?.doneJobs || 0)),
    startedAt: String(run?.startedAt || ""),
    updatedAt: String(run?.updatedAt || ""),
    finishedAt: String(run?.finishedAt || ""),
    lastBatch: run?.lastBatch && typeof run.lastBatch === "object" ? run.lastBatch : null,
    activeBatch: run?.activeBatch && typeof run.activeBatch === "object" ? run.activeBatch : null,
    stopRequested: Boolean(run?.stopRequested),
  };
}

export function sourceTreeImportPlan(rows = [], options = {}) {
  const alreadySet = new Set(uniqueTexts(options.alreadyIngestedSourceIds || []));
  const limit = Number.isFinite(Number(options.limit)) ? Math.max(0, Number(options.limit)) : 0;
  const pending = rows.filter((row) => {
    const sourceId = amazonSourceId(row);
    return sourceId && !alreadySet.has(sourceId);
  });
  return {
    total: rows.length,
    alreadyIngested: rows.length - pending.length,
    toImport: limit > 0 ? pending.slice(0, limit) : pending,
  };
}

export function sourceTreeSearchTerms(query = "", limit = 12) {
  const value = String(query || "");
  const lower = value.toLowerCase();
  const terms = [];
  const add = (...items) => {
    for (const item of items) {
      const term = String(item || "").trim();
      if (term && !terms.includes(term)) terms.push(term);
    }
  };

  if (/主图|图片|视觉|点击率|转化率/.test(value)) add("主图", "点击率", "转化率", "图片", "视觉", "Listing", "页面");
  if (/广告|推广|投放|acos|cpc/i.test(value)) add("广告", "推广", "投放", "ACOS", "CPC", "关键词");
  if (/listing|文案|关键词|收录|标题|search term|五点|bullet/i.test(value)) add("Listing", "文案", "关键词", "收录", "标题", "Search Term", "五点");
  if (/选品|产品|值不值得|市场|竞争|利润|差异化/.test(value)) add("选品", "产品", "市场", "竞争", "利润", "差异化");

  for (const keyword of SOURCE_TREE_KNOWN_TERMS) {
    if (value.includes(keyword) || lower.includes(keyword.toLowerCase())) add(keyword);
  }
  for (const match of value.matchAll(/[a-zA-Z][a-zA-Z0-9_-]{1,24}/g)) {
    const token = match[0];
    if (!SOURCE_TREE_STOP_WORDS.has(token.toLowerCase())) add(token);
  }

  return uniqueTexts(terms).slice(0, Math.max(1, Number(limit) || 12));
}

export function buildSourceTreeCalibration(input = {}) {
  const terms = uniqueTexts(input.terms || []).slice(0, 12);
  const rawCandidates = Array.isArray(input.chunkRows) ? input.chunkRows : [];
  const rawSummaries = Array.isArray(input.summaryRows) ? input.summaryRows : [];
  const resolvedSources = Array.isArray(input.resolvedSources) ? input.resolvedSources : [];
  const resolvedByKey = new Map();
  for (const source of resolvedSources) {
    const normalized = normalizeSourceTreeSource(source);
    if (!normalized.sourceId && !normalized.sourceRef && !normalized.owner) continue;
    resolvedByKey.set(sourceTreeCandidateKey(normalized), normalized);
  }

  const grouped = new Map();
  for (const row of rawCandidates) {
    const normalized = normalizeSourceTreeSource(row);
    if (!normalized.sourceId && !normalized.sourceRef) continue;
    const key = sourceTreeCandidateKey(normalized);
    const existing = grouped.get(key) || {
      ...normalized,
      chunkCount: 0,
      matchScore: 0,
    };
    existing.chunkCount += Math.max(1, Number(row?.chunkCount || row?.chunk_count || 1));
    existing.matchScore = Math.max(existing.matchScore, Number(row?.matchScore || row?.match_score || 0));
    grouped.set(key, existing);
  }

  const candidates = [...grouped.entries()]
    .map(([key, item], index) => {
      const resolved = resolvedByKey.get(key);
      const title = firstText(resolved?.title, item.title, titleFromSourceId(item.sourceId), item.sourceRef, item.sourceId);
      return {
        id: `source-tree:candidate:${index}`,
        type: "route_hint",
        label: title || "来源树候选原文",
        owner: item.owner,
        sourceId: item.sourceId,
        sourceRef: item.sourceRef,
        chunkCount: item.chunkCount,
        matchScore: item.matchScore,
        matchedOriginalSource: Boolean(resolved),
        matchedTitle: resolved?.title || "",
        canUseAsEvidence: false,
      };
    })
    .sort((a, b) => Number(b.matchedOriginalSource) - Number(a.matchedOriginalSource) || b.matchScore - a.matchScore || b.chunkCount - a.chunkCount)
    .slice(0, 8);

  const summaries = rawSummaries
    .map((row, index) => ({
      id: firstText(row?.id, `source-tree:summary:${index}`),
      type: "summary_hint",
      label: compactSourceTreeText(row?.title || row?.treeId || row?.tree_id || `摘要提示 ${index + 1}`, 80),
      treeId: firstText(row?.treeId, row?.tree_id),
      excerpt: compactSourceTreeText(row?.content, 220),
      canUseAsEvidence: false,
    }))
    .filter((item) => item.excerpt || item.label)
    .slice(0, 4);

  const candidateCount = candidates.length;
  const resolvedSourceCount = candidates.filter((item) => item.matchedOriginalSource).length;
  const summaryHintCount = summaries.length;
  const status = candidateCount > 0
    ? resolvedSourceCount > 0 ? "active" : "unresolved"
    : summaryHintCount > 0 ? "summary_only" : "empty";
  const summary = status === "active"
    ? `OpenHuman 来源树辅助命中 ${candidateCount} 个候选来源，其中 ${resolvedSourceCount} 个已回到作者原文库核对。`
    : status === "unresolved"
      ? `OpenHuman 来源树辅助命中 ${candidateCount} 个候选来源，但本轮还没有回到完整作者原文。`
      : status === "summary_only"
        ? `OpenHuman 来源树只提供了 ${summaryHintCount} 条摘要提示，本轮没有把摘要当证据使用。`
        : "本轮没有用到 OpenHuman 来源树候选，回答仍只看普通语义检索和可引用原文。";

  return {
    title: "OpenHuman 来源树辅助检索",
    status,
    terms,
    candidateCount,
    resolvedSourceCount,
    summaryHintCount,
    summary,
    boundary: "来源树摘要和候选片段只负责帮系统找路，不能当作者原文证据；回答里的引用必须回到本地作者原文上下文后才可采纳。",
    candidates,
    summaries,
  };
}

export function articleDateToMs(value) {
  const raw = String(value || "").trim();
  if (!raw) return Date.now();
  const normalized = /^\d{4}-\d{2}-\d{2}$/.test(raw) ? `${raw}T00:00:00.000Z` : raw;
  const parsed = Date.parse(normalized);
  return Number.isFinite(parsed) ? parsed : Date.now();
}

function intersectStableIds(values = [], allowed = []) {
  const allowedSet = new Set(uniqueTexts(allowed));
  return uniqueTexts(values).filter((value) => allowedSet.has(value));
}

function uniqueTexts(values = []) {
  return [...new Set(values.map((value) => String(value || "").trim()).filter(Boolean))];
}

function firstText(...values) {
  return values.map((value) => String(value || "").trim()).find(Boolean) || "";
}

const SOURCE_TREE_KNOWN_TERMS = [
  "主图",
  "副图",
  "点击率",
  "转化率",
  "Listing",
  "广告",
  "关键词",
  "标题",
  "五点",
  "收录",
  "选品",
  "竞品",
  "利润",
  "差异化",
  "评价",
  "价格",
  "页面",
  "图片",
  "视觉",
];

const SOURCE_TREE_STOP_WORDS = new Set(["the", "and", "for", "with", "this", "that", "listing"]);

function normalizeSourceTreeSource(row = {}) {
  return {
    sourceId: firstText(row.sourceId, row.source_id),
    sourceRef: firstText(row.sourceRef, row.source_ref, row.sourceUrl, row.source_url),
    owner: firstText(row.owner, row.author),
    title: firstText(row.title, row.matchedTitle, row.matched_title),
  };
}

function sourceTreeCandidateKey(source = {}) {
  return [source.sourceId || "", source.sourceRef || "", source.owner || ""].join("|");
}

function titleFromSourceId(sourceId = "") {
  const base = String(sourceId || "").split(/[\\/]/).pop() || "";
  return base.replace(/\.html?$/i, "").replace(/^[^_]+_\d{4}-\d{2}-\d{2}_/, "").trim();
}

function compactSourceTreeText(value, maxLength = 160) {
  const text = String(value || "").replace(/\s+/g, " ").trim();
  if (!text) return "";
  return text.length > maxLength ? `${text.slice(0, Math.max(0, maxLength - 1))}…` : text;
}

function formatDrainNumber(value) {
  const number = Number(value || 0);
  return Number.isFinite(number) ? number.toLocaleString("zh-CN") : "0";
}

function formatDrainDuration(minutes) {
  const value = Math.max(1, Math.round(Number(minutes || 0)));
  if (value < 60) return `${formatDrainNumber(value)} 分钟`;
  const hours = Math.floor(value / 60);
  const rest = value % 60;
  if (hours < 24) {
    return rest > 0
      ? `${formatDrainNumber(hours)} 小时 ${formatDrainNumber(rest)} 分钟`
      : `${formatDrainNumber(hours)} 小时`;
  }
  const days = Math.floor(hours / 24);
  const restHours = hours % 24;
  return restHours > 0
    ? `${formatDrainNumber(days)} 天 ${formatDrainNumber(restHours)} 小时`
    : `${formatDrainNumber(days)} 天`;
}
