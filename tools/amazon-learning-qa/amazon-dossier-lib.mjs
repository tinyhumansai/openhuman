export function buildLearningDossier(input = {}) {
  const message = input.message && typeof input.message === "object" ? input.message : {};
  const card = message.learningCard && typeof message.learningCard === "object" ? message.learningCard : {};
  const feedback = message.evidenceFeedback && typeof message.evidenceFeedback === "object" ? message.evidenceFeedback : {};
  const sources = Array.isArray(message.sources) ? message.sources : [];
  const sourceControls = input.sourceControls && typeof input.sourceControls === "object" ? input.sourceControls : {};
  const claims = Array.isArray(message.evidenceChain?.claims) ? message.evidenceChain.claims : [];
  const sourceLabels = new Map();
  const excludedSourceKeys = normalizeSourceKeys(sourceControls.excludedSourceKeys);
  const excludedSourceKeySet = new Set(excludedSourceKeys);
  const sourceIsExcluded = (source) => sourceIdentityKeys(normalizeSource(source)).some((key) => excludedSourceKeySet.has(key));

  sources.forEach((source) => {
    const normalized = normalizeSource(source);
    sourceIdentityKeys(normalized).forEach((key) => sourceLabels.set(key, sourceLabel(normalized)));
  });

  const acceptedEvidence = [];
  const rejectedEvidence = [];
  claims
    .filter((claim) => claim?.type === "source_evidence")
    .forEach((claim) => {
      const value = feedback[claim.id];
      if (value !== "useful" && value !== "irrelevant") return;
      if (!isValidSourceEvidenceClaim(claim, sources)) return;
      const source = sources[claim.sourceIndex];
      if (sourceIsExcluded(source)) return;
      const evidence = normalizeEvidenceClaim(claim, source);
      if (value === "useful") acceptedEvidence.push(evidence);
      if (value === "irrelevant") rejectedEvidence.push(evidence);
    });

  const excludedSources = excludedSourceKeys.map((key) => ({
    key,
    label: sourceLabels.get(key) || compactSourceKey(key),
  }));
  const allowedAuthors = normalizeAuthorNames(sourceControls.allowedAuthors);

  const question = sanitizeText(input.question || message.question || card.question || inferQuestion(message.content), 220);
  const productInputSummary = normalizeProductInputSummary(message.productInputSummary);
  const diagnosisPanel = normalizeDiagnosisPanel(message.diagnosisPanel);
  const validationPack = normalizeValidationPack(message.validationPack);
  const synthesisAnswer = normalizeSynthesisAnswer(message.synthesisAnswer, sources);
  const workflowIntent = normalizeWorkflowIntent(message.workflowIntent);
  return {
    id: sanitizeText(input.id || "", 80),
    createdAt: sanitizeText(input.createdAt || new Date().toISOString(), 40),
    title: sanitizeText(workflowIntent?.label || card.intent?.label || question || "亚马逊学习档案", 120),
    question,
    takeaway: sanitizeText(card.takeaway || firstMeaningfulLine(message.content), 360),
    answerPreview: sanitizeText(stripInlineMarkers(message.content || ""), 900),
    conclusions: sanitizeList(card.conclusions, 6, 260),
    nextActions: sanitizeList(card.nextActions, 8, 260),
    missingInputs: sanitizeList(card.missingInputs, 8, 260),
    followUps: sanitizeList(card.followUps, 6, 220),
    acceptedEvidence: acceptedEvidence.slice(0, 12),
    rejectedEvidence: rejectedEvidence.slice(0, 12),
    excludedSources: excludedSources.slice(0, 30),
    allowedAuthors,
    sources: sources.map(normalizeSource).filter((source) => source.title && !sourceIsExcluded(source)).slice(0, 12),
    productInputSummary,
    diagnosisPanel,
    validationPack,
    synthesisAnswer,
    workflowIntent,
  };
}

export function validateLearningDossierForSave(dossier = {}, input = {}) {
  const normalized = normalizeStoredDossier(dossier);
  const feedback = input?.message?.evidenceAudit?.feedback || input?.evidenceAudit?.feedback || "";
  if (feedback === "citation_wrong" || feedback === "retry") {
    return {
      ok: false,
      code: "answer_needs_recheck",
      message: "这轮回答已被标记为引用不准或需要重查，请先重新核对来源后再保存学习档案。",
    };
  }
  if (normalized.acceptedEvidence.length === 0) {
    return {
      ok: false,
      code: "needs_accepted_evidence",
      message: "保存学习档案前，必须先把至少一条未被排除的作者原文证据标记为有用。",
    };
  }
  if (input?.requireSourceAuthenticity === true) {
    const verifiedSourceKeys = normalizeSourceKeys(input.verifiedSourceKeys);
    const verifiedKeySet = new Set(verifiedSourceKeys);
    const unverified = normalized.acceptedEvidence.filter(
      (item) => !evidenceIdentityKeys(item).some((key) => verifiedKeySet.has(key)),
    );
    if (unverified.length > 0) {
      return {
        ok: false,
        code: "unverified_source_identity",
        message: "保存学习档案前，必须确认已采纳证据能回到本地原文库；当前有来源身份无法核验。",
      };
    }
  }
  return { ok: true, code: "ok", message: "" };
}

function isValidSourceEvidenceClaim(claim, sources) {
  if (!Number.isInteger(claim?.sourceIndex)) return false;
  const source = sources[claim.sourceIndex];
  if (!source || typeof source !== "object") return false;
  const normalized = normalizeSource(source);
  if (!isAuthorEvidenceSource(normalized)) return false;
  if (!sourceKeyForSource(normalized) || !normalized.title) return false;
  const quote = sanitizeText(claim.quote || claim.text, 700);
  if (!quote) return false;
  if (!sourceContainsQuote(normalized, quote)) return false;
  return true;
}

export function normalizeStoredDossier(value = {}) {
  const sources = Array.isArray(value.sources) ? value.sources.map(normalizeSource).filter((source) => source.title).slice(0, 12) : [];
  const title = sanitizeText(value.title || "亚马逊学习档案", 120);
  const question = sanitizeText(value.question, 220);
  const takeaway = sanitizeText(value.takeaway, 360);
  const answerPreview = sanitizeText(value.answerPreview, 900);
  const conclusions = sanitizeList(value.conclusions, 6, 260);
  const nextActions = sanitizeList(value.nextActions, 8, 260);
  const missingInputs = sanitizeList(value.missingInputs, 8, 260);
  const acceptedEvidence = normalizeEvidenceList(value.acceptedEvidence, 12);
  const synthesisAnswer = normalizeSynthesisAnswer(value.synthesisAnswer, sources)
    || buildFallbackSynthesisAnswer({
      title,
      question,
      takeaway,
      answerPreview,
      conclusions,
      nextActions,
      missingInputs,
      acceptedEvidence,
      sources,
    });
  return {
    id: sanitizeText(value.id, 80),
    createdAt: sanitizeText(value.createdAt, 40),
    title,
    question,
    takeaway,
    answerPreview,
    conclusions,
    nextActions,
    missingInputs,
    followUps: sanitizeList(value.followUps, 6, 220),
    acceptedEvidence,
    rejectedEvidence: normalizeEvidenceList(value.rejectedEvidence, 12),
    excludedSources: normalizeExcludedSources(value.excludedSources, 30),
    allowedAuthors: normalizeAuthorNames(value.allowedAuthors),
    sources,
    productInputSummary: normalizeProductInputSummary(value.productInputSummary),
    diagnosisPanel: normalizeDiagnosisPanel(value.diagnosisPanel),
    validationPack: normalizeValidationPack(value.validationPack),
    synthesisAnswer,
    workflowIntent: normalizeWorkflowIntent(value.workflowIntent),
    reviewState: normalizeReviewState(value.reviewState),
    selfTestState: normalizeSelfTestState(value.selfTestState),
    businessVerificationRecords: normalizeBusinessVerificationRecords(value.businessVerificationRecords),
    experimentResultRecords: normalizeExperimentResultRecords(value.experimentResultRecords),
    openhumanMemory: normalizeOpenHumanMemoryRecord(value.openhumanMemory),
  };
}

export function updateDossierEvidenceDecisionState(value = {}, input = {}) {
  const dossier = normalizeStoredDossier(value);
  const sourceIndex = Number(input.sourceIndex);
  const decision = input.decision === "useful" || input.decision === "irrelevant" ? input.decision : "";
  if (!Number.isInteger(sourceIndex) || sourceIndex < 0 || !decision) return dossier;
  const source = dossier.sources[sourceIndex];
  if (!source || !source.title || !source.excerpt) return dossier;
  const evidence = evidenceFromDossierSource(source);
  if (!evidence.quote || !evidence.sourceKey) return dossier;
  const sourceKeys = new Set(sourceIdentityKeys(source));
  const keepOtherSource = (item) => !evidenceIdentityKeys(item).some((key) => sourceKeys.has(key));
  const acceptedEvidence = dossier.acceptedEvidence.filter(keepOtherSource);
  const rejectedEvidence = dossier.rejectedEvidence.filter(keepOtherSource);
  if (decision === "useful") acceptedEvidence.unshift(evidence);
  if (decision === "irrelevant") rejectedEvidence.unshift(evidence);
  return normalizeStoredDossier({
    ...dossier,
    acceptedEvidence: acceptedEvidence.slice(0, 12),
    rejectedEvidence: rejectedEvidence.slice(0, 12),
  });
}

function evidenceFromDossierSource(source) {
  const normalizedSource = normalizeSource(source);
  return {
    id: sanitizeText(`source:${sourceKeyForSource(normalizedSource)}`, 80),
    quote: sanitizeText(normalizedSource.excerpt, 700),
    text: sanitizeText(normalizedSource.excerpt, 260),
    author: normalizedSource.author,
    title: normalizedSource.title,
    date: normalizedSource.date,
    sourceUrl: normalizedSource.sourceUrl,
    sourcePath: normalizedSource.sourcePath,
    sourceKey: sourceKeyForSource(normalizedSource),
  };
}

export function buildOpenHumanMemoryDocument(value = {}, options = {}) {
  const dossier = normalizeStoredDossier(value);
  const sourceNamespace = sanitizeText(options.sourceNamespace || "amazon-learning", 80) || "amazon-learning";
  const namespace = sanitizeText(options.namespace || `${sourceNamespace}-workflow`, 80) || `${sourceNamespace}-workflow`;
  const key = `dossier/${dossier.id || stableHash(`${dossier.createdAt}:${dossier.title}`)}`;
  const title = sanitizeText(`亚马逊学习档案：${dossier.title || dossier.question || "未命名档案"}`, 160);
  const content = buildOpenHumanMemoryContent(dossier, sourceNamespace);
  return {
    namespace,
    key,
    title,
    content,
    source_type: "amazon-learning-dossier",
    priority: "high",
    category: "amazon-learning-workflow",
    tags: [
      "amazon",
      "亚马逊学习",
      "openhuman-evidence-workflow",
      "learning-dossier",
      ...dossier.allowedAuthors.slice(0, 3),
    ],
    metadata: {
      dossier_id: dossier.id,
      source_namespace: sourceNamespace,
      accepted_evidence_count: dossier.acceptedEvidence.length,
      rejected_evidence_count: dossier.rejectedEvidence.length,
      excluded_source_count: dossier.excludedSources.length,
      business_verification_records: dossier.businessVerificationRecords.length,
      experiment_results: dossier.experimentResultRecords.length,
      validation_pack_status: dossier.validationPack?.status || "",
      synthesis_status: dossier.synthesisAnswer?.status || "",
      created_at: dossier.createdAt,
      boundary: "用户业务材料不是作者原文证据；原作者资料仍保留在 source_namespace 中。",
    },
  };
}

function normalizeOpenHumanMemoryRecord(value = {}) {
  const record = value && typeof value === "object" ? value : {};
  const status = ["not_synced", "pending", "synced", "failed", "skipped"].includes(record.status)
    ? record.status
    : "not_synced";
  return {
    status,
    namespace: sanitizeText(record.namespace, 80),
    key: sanitizeText(record.key, 180),
    documentId: sanitizeText(record.documentId || record.document_id, 120),
    documentTitle: sanitizeText(record.documentTitle || record.document_title, 180),
    syncedAt: sanitizeText(record.syncedAt || record.synced_at, 40),
    error: sanitizeText(record.error || record.message, 260),
    indexStatus: sanitizeText(record.indexStatus || record.index_status, 40),
    totalChunks: Number.isFinite(Number(record.totalChunks ?? record.total_chunks)) ? Math.max(0, Number(record.totalChunks ?? record.total_chunks)) : 0,
    indexedChunks: Number.isFinite(Number(record.indexedChunks ?? record.indexed_chunks)) ? Math.max(0, Number(record.indexedChunks ?? record.indexed_chunks)) : 0,
  };
}

function buildOpenHumanMemoryContent(dossier, sourceNamespace) {
  const traceableAcceptedEvidence = traceableEvidenceList(dossier.acceptedEvidence);
  const conclusionRows = uniqueOrdered(dossier.conclusions, 8);
  const supportedConclusions = conclusionRows.filter((item) =>
    !isBusinessSpecificClaim(item) &&
    matchedEvidenceForClaim(item, traceableAcceptedEvidence).length > 0
  );
  const pendingConclusions = conclusionRows.filter((item) => !supportedConclusions.includes(item));
  const lines = [
    `# 亚马逊学习档案：${dossier.title || dossier.question || "未命名档案"}`,
    "",
    `来源命名空间：${sourceNamespace}`,
    `档案编号：${dossier.id}`,
    `创建时间：${dossier.createdAt}`,
    "",
    "## 问题",
    dossier.question || "未记录问题",
    "",
    "## 当前理解状态",
    `证据状态：${traceableAcceptedEvidence.length > 0 ? `已有 ${traceableAcceptedEvidence.length} 条已采纳原文证据` : "暂无已采纳原文证据"}`,
    dossier.takeaway || "暂无结论",
  ];

  if (supportedConclusions.length > 0) {
    lines.push("", "## 有来源支撑的结论");
    supportedConclusions.slice(0, 6).forEach((item, index) => lines.push(`${index + 1}. ${item}`));
  }

  if (pendingConclusions.length > 0 || (traceableAcceptedEvidence.length === 0 && dossier.takeaway)) {
    lines.push("", "## 待验证理解");
    const pendingRows = uniqueOrdered([
      ...(pendingConclusions.length > 0 ? pendingConclusions : []),
      traceableAcceptedEvidence.length === 0 ? dossier.takeaway : "",
    ], 6);
    pendingRows.forEach((item, index) => lines.push(`${index + 1}. ${item}`));
  }

  if (dossier.nextActions.length > 0) {
    lines.push("", "## 下一步行动");
    dossier.nextActions.slice(0, 8).forEach((item, index) => lines.push(`${index + 1}. ${item}`));
  }

  if (dossier.synthesisAnswer) {
    lines.push(
      "",
      "## 本轮综合讲义",
      `状态：${synthesisStatusLabel(dossier.synthesisAnswer.status)}`,
      dossier.synthesisAnswer.summary || "这是根据本轮已定位来源整理的学习讲义。",
      "系统综合不是作者原文证据；只有下方绑定的作者摘录可以作为来源支撑。",
    );
    dossier.synthesisAnswer.points.slice(0, 6).forEach((point, index) => {
      lines.push(`${index + 1}. ${point.label || "综合要点"}${point.text ? `：${point.text}` : ""}`);
      if (point.support.length > 0) {
        point.support.slice(0, 3).forEach((item) => {
          const source = [item.author, item.title].filter(Boolean).join("《") + (item.author && item.title ? "》" : "");
          lines.push(`   支撑：${item.quote || source || "已绑定来源"}`);
        });
      }
    });
    if (dossier.synthesisAnswer.gaps.length > 0) {
      lines.push("", "### 讲义缺口");
      dossier.synthesisAnswer.gaps.slice(0, 4).forEach((item, index) => lines.push(`${index + 1}. ${item.label}${item.reason ? `：${item.reason}` : ""}`));
    }
  }

  lines.push("", "## 已采纳原文证据");
  if (dossier.acceptedEvidence.length === 0) {
    lines.push("暂无。");
  } else {
    dossier.acceptedEvidence.slice(0, 8).forEach((item, index) => {
      lines.push(`${index + 1}. ${item.quote || item.text}（${item.author}《${item.title}》${item.date ? `，${item.date}` : ""}）`);
      if (item.sourceUrl) lines.push(`   原文：${item.sourceUrl}`);
      if (item.sourcePath) lines.push(`   来源文件：${item.sourcePath}`);
    });
  }

  if (dossier.rejectedEvidence.length > 0) {
    lines.push("", "## 用户排除的证据");
    dossier.rejectedEvidence.slice(0, 6).forEach((item, index) => {
      lines.push(`${index + 1}. ${item.quote || item.text}（${item.author}《${item.title}》）`);
    });
  }

  if (dossier.excludedSources.length > 0) {
    lines.push("", "## 用户排除的来源");
    dossier.excludedSources.slice(0, 12).forEach((item, index) => lines.push(`${index + 1}. ${item.label || item.key}`));
  }

  if (dossier.productInputSummary?.facts?.length > 0) {
    lines.push("", "## 用户产品输入");
    dossier.productInputSummary.facts.slice(0, 8).forEach((section) => {
      lines.push(`### ${section.label}`);
      (section.items || []).slice(0, 6).forEach((item) => lines.push(`- ${item}`));
      if ((section.missing || []).length > 0) lines.push(`- 仍缺：${section.missing.join("、")}`);
    });
  }

  if (dossier.validationPack) {
    lines.push(
      "",
      "## 业务验证任务包",
      `状态：${validationSourceStatusLabel(dossier.validationPack.status)}`,
      dossier.validationPack.summary || "把本轮回答推进成可验证的业务动作。",
      "任务包不是作者原文证据；它只记录下一步要补的数据、实验和判断规则。",
    );
    if (dossier.validationPack.hypotheses.length > 0) {
      lines.push("", "### 待验证假设");
      dossier.validationPack.hypotheses.slice(0, 4).forEach((item, index) => {
        const source = [item.author, item.sourceTitle].filter(Boolean).join("《") + (item.author && item.sourceTitle ? "》" : "");
        lines.push(`${index + 1}. ${item.label || item.quote}${source ? `（${source}）` : ""}`);
      });
    }
    if (dossier.validationPack.dataRequests.length > 0) {
      lines.push("", "### 要补的数据");
      dossier.validationPack.dataRequests.slice(0, 6).forEach((item, index) => {
        lines.push(`${index + 1}. ${item.label}${item.why ? `：${item.why}` : ""}`);
      });
    }
    if (dossier.validationPack.experiments.length > 0) {
      lines.push("", "### 建议小实验");
      dossier.validationPack.experiments.slice(0, 3).forEach((item, index) => {
        lines.push(`${index + 1}. ${item.title}${item.steps.length ? `：${item.steps.join(" / ")}` : ""}`);
        if (item.successSignal) lines.push(`   观察：${item.successSignal}`);
      });
    }
    if (dossier.validationPack.decisionRules.length > 0) {
      lines.push("", "### 判断规则");
      dossier.validationPack.decisionRules.slice(0, 5).forEach((item, index) => {
        lines.push(`${index + 1}. 如果 ${item.if}，则 ${item.then}`);
      });
    }
  }

  if (dossier.businessVerificationRecords.length > 0) {
    lines.push("", "## 用户业务材料");
    dossier.businessVerificationRecords.slice(0, 6).forEach((record, index) => {
      lines.push(`${index + 1}. ${record.summary || record.rawText}`);
      if (record.rawText) lines.push(`   原始材料：${record.rawText}`);
    });
  }

  if (dossier.experimentResultRecords.length > 0) {
    lines.push("", "## 实验复盘");
    dossier.experimentResultRecords.slice(0, 6).forEach((record, index) => {
      lines.push(`${index + 1}. ${record.summary || record.rawText}`);
      if (record.nextAction) lines.push(`   下一步：${record.nextAction}`);
    });
  }

  lines.push(
    "",
    "## 证据边界",
    "用户业务材料不是作者原文证据；采纳和排除只代表用户当前判断。继续回答时应区分作者原文、系统推断和用户业务验证。",
  );
  return lines.join("\n");
}

export function buildDossierWorkbench(value = {}) {
  const dossier = normalizeStoredDossier(value);
  const checklist = [];
  const addChecklistItem = (kind, label, reason, prompt) => {
    const safeLabel = sanitizeText(label, 180);
    if (!safeLabel) return;
    checklist.push({
      id: `${kind}:${checklist.filter((item) => item.kind === kind).length}`,
      kind,
      label: safeLabel,
      reason: sanitizeText(reason, 180),
      prompt: sanitizeText(prompt, 360),
    });
  };

  dossier.nextActions.slice(0, 6).forEach((item) => {
    addChecklistItem(
      "action",
      item,
      dossier.takeaway || "来自学习档案的行动顺序",
      `我正在复盘“${item}”，请基于这个学习档案判断下一步应该验证什么。`,
    );
  });

  dossier.missingInputs.slice(0, 6).forEach((item) => {
    addChecklistItem(
      "input",
      `补充：${item}`,
      "这个信息会让下一轮判断更贴近你的真实产品。",
      `我补充了${item}：。请基于这个学习档案重新诊断，并告诉我先看什么。`,
    );
  });

  dossier.acceptedEvidence.slice(0, 3).forEach((item) => {
    const label = item.title ? `用已采纳证据复核：${item.title}` : "用已采纳证据复核页面判断";
    addChecklistItem(
      "evidence",
      label,
      item.text || item.quote || "来自已采纳的原文证据",
      `请用这条已采纳证据复核我的页面判断：${item.text || item.quote || ""}`,
    );
  });

  const questionPack = buildQuestionPack(dossier);
  const excludedKeys = new Set(dossier.excludedSources.map((item) => item.key).filter(Boolean));
  const acceptedFromExcluded = dossier.acceptedEvidence.some((item) => excludedKeys.has(item.sourceKey));
  const diagnosis = buildDossierDiagnosisSummary(dossier);
  const reviewQueue = buildReviewQueue(dossier);
  const selfTest = buildSelfTest(dossier);
  const businessVerification = buildBusinessVerificationPanel(dossier);
  const validationPack = buildDossierValidationPackProgress(dossier);
  const synthesisGuide = buildDossierSynthesisGuide(dossier);

  return {
    title: dossier.title || "学习工作台",
    summary: sanitizeText(dossier.takeaway || "把这个档案转成可执行的检查清单和追问包。", 260),
    checklist: checklist.slice(0, 12),
    questionPack,
    evidencePolicy: {
      acceptedEvidence: dossier.acceptedEvidence.length,
      rejectedEvidence: dossier.rejectedEvidence.length,
      excludedSources: dossier.excludedSources.length,
      acceptedFromExcluded,
      message: acceptedFromExcluded
        ? "保存时有采纳证据来自已排除来源；继续追问会保留采纳证据快照，同时避开被排除的整篇来源。"
        : "继续追问会优先参考已采纳证据，并避开已排除来源。",
    },
    diagnosis,
    reviewQueue,
    selfTest,
    businessVerification,
    validationPack,
    synthesisGuide,
  };
}

export function buildDossierOverview(values = []) {
  const dossiers = Array.isArray(values)
    ? values.map(normalizeStoredDossier).filter((dossier) => dossier.id)
    : [];
  const summaries = dossiers.map((dossier) => {
    const workbench = buildDossierWorkbench(dossier);
    const queue = workbench.reviewQueue || buildReviewQueue(dossier);
    const selfTest = workbench.selfTest || buildSelfTest(dossier);
    const nextItem = queue.nextItem ? {
      ...queue.nextItem,
      dossierId: dossier.id,
      dossierTitle: dossier.title,
      dossierQuestion: dossier.question,
      createdAt: dossier.createdAt,
    } : null;
    return {
      dossier,
      queue,
      selfTest,
      businessVerification: workbench.businessVerification || buildBusinessVerificationPanel(dossier),
      nextItem,
      progress: queue.progress || { completed: 0, total: 0, percent: 0 },
      selfTestProgress: selfTest.progress || { mastered: 0, total: 0, percent: 0 },
    };
  });

  const totals = summaries.reduce((acc, item) => {
    acc.reviewTotal += item.progress.total || 0;
    acc.reviewCompleted += item.progress.completed || 0;
    acc.selfTestTotal += item.selfTestProgress.total || 0;
    acc.selfTestMastered += item.selfTestProgress.mastered || 0;
    acc.businessVerificationRecords += item.dossier.businessVerificationRecords.length;
    acc.businessVerificationReady += item.dossier.businessVerificationRecords.filter((record) => record.status === "ready").length;
    if ((item.businessVerification?.totalRecords || 0) > 0) {
      acc.businessVerificationDimensionsReady += item.businessVerification.coverage?.ready || 0;
      acc.businessVerificationDimensionsTotal += item.businessVerification.coverage?.total || 0;
    }
    acc.experimentResults += item.dossier.experimentResultRecords.length;
    acc.experimentResultsPositive += item.dossier.experimentResultRecords.filter((record) => record.outcome === "positive").length;
    acc.acceptedEvidence += item.dossier.acceptedEvidence.length;
    acc.rejectedEvidence += item.dossier.rejectedEvidence.length;
    acc.excludedSources += item.dossier.excludedSources.length;
    if (item.dossier.acceptedEvidence.length === 0) acc.dossiersWithoutAcceptedEvidence += 1;
    return acc;
  }, {
    dossiers: summaries.length,
    reviewTotal: 0,
    reviewCompleted: 0,
    selfTestTotal: 0,
    selfTestMastered: 0,
    businessVerificationRecords: 0,
    businessVerificationReady: 0,
    businessVerificationDimensionsReady: 0,
    businessVerificationDimensionsTotal: 0,
    experimentResults: 0,
    experimentResultsPositive: 0,
    acceptedEvidence: 0,
    rejectedEvidence: 0,
    excludedSources: 0,
    dossiersWithoutAcceptedEvidence: 0,
  });
  totals.reviewOpen = Math.max(0, totals.reviewTotal - totals.reviewCompleted);
  totals.reviewPercent = totals.reviewTotal > 0 ? Math.round((totals.reviewCompleted / totals.reviewTotal) * 100) : 0;
  totals.selfTestOpen = Math.max(0, totals.selfTestTotal - totals.selfTestMastered);
  totals.selfTestPercent = totals.selfTestTotal > 0 ? Math.round((totals.selfTestMastered / totals.selfTestTotal) * 100) : 0;
  totals.businessVerificationOpen = summaries.filter((item) => item.dossier.businessVerificationRecords.length === 0).length;

  const nextItems = summaries
    .map((item) => item.nextItem)
    .filter(Boolean)
    .sort((a, b) => reviewKindPriority(a.kind) - reviewKindPriority(b.kind) || String(b.createdAt).localeCompare(String(a.createdAt)))
    .slice(0, 8);

  const focusDossiers = summaries
    .filter((item) => item.progress.total > 0)
    .sort((a, b) => (a.progress.percent || 0) - (b.progress.percent || 0) || String(b.dossier.createdAt).localeCompare(String(a.dossier.createdAt)))
    .slice(0, 6)
    .map((item) => ({
      id: item.dossier.id,
      title: item.dossier.title,
      question: item.dossier.question,
      takeaway: item.dossier.takeaway,
      createdAt: item.dossier.createdAt,
      progress: item.progress,
      selfTestProgress: item.selfTestProgress,
      nextItem: item.nextItem ? {
        id: item.nextItem.id,
        kind: item.nextItem.kind,
        label: item.nextItem.label,
        prompt: item.nextItem.prompt,
      } : null,
    }));

  const researchMissions = summaries
    .map(buildResearchMission)
    .filter(Boolean)
    .sort((a, b) => missionStagePriority(a.stage) - missionStagePriority(b.stage) || String(b.createdAt).localeCompare(String(a.createdAt)))
    .slice(0, 8);
  const studyMaterials = buildOverviewStudyMaterials(summaries, totals);
  const topicGroups = buildResearchTopicGroups(summaries);
  const learningProducts = buildOverviewLearningProducts(summaries, totals, topicGroups);
  const learningPaths = buildOverviewLearningPaths(topicGroups, researchMissions);
  const mastery = buildOverviewMastery(totals, topicGroups, learningPaths);

  const evidenceGaps = summaries
    .filter((item) => item.dossier.acceptedEvidence.length === 0 || item.dossier.sources.length === 0 || item.dossier.missingInputs.length > 0)
    .slice(0, 8)
    .map((item) => ({
      id: item.dossier.id,
      title: item.dossier.title,
      question: item.dossier.question,
      reason: item.dossier.acceptedEvidence.length === 0
        ? "缺少原文证据采纳，继续追问前建议先补来源或标记有用证据。"
        : item.dossier.sources.length === 0
          ? "缺少原始来源身份，建议补充更具体的问题或产品材料。"
          : "仍有待补材料，建议先补齐后再复盘。",
      missingInputs: item.dossier.missingInputs.slice(0, 5),
    }));

  return {
    summary: totals.dossiers > 0
      ? `${totals.dossiers} 个学习档案，已处理 ${totals.reviewCompleted}/${totals.reviewTotal} 个推进项，自测理解 ${totals.selfTestMastered}/${totals.selfTestTotal} 张，已保存 ${totals.businessVerificationRecords} 条业务材料，仍有 ${totals.businessVerificationOpen} 个档案未补产品材料。`
      : "还没有学习档案。先保存一次知识库回答，就能生成学习路径。",
    totals,
    nextItems,
    focusDossiers,
    researchMissions,
    studyMaterials,
    mastery,
    topicGroups,
    learningPaths,
    learningProducts,
    evidenceGaps,
  };
}

function buildOverviewMastery(totals = {}, topicGroups = [], learningPaths = []) {
  const groups = Array.isArray(topicGroups) ? topicGroups : [];
  const paths = Array.isArray(learningPaths) ? learningPaths : [];
  const topicTotal = groups.length;
  const sourceReady = groups.filter((group) => Number(group.evidenceCount || 0) > 0).length;
  const materialReady = groups.filter((group) => Number(group.materialRecords || 0) > 0).length;
  const experimentReady = groups.filter((group) => Number(group.experimentResults || 0) > 0).length;
  const reviewPercent = Math.max(0, Math.min(100, Number(totals.reviewPercent || 0)));
  const selfTestPercent = Math.max(0, Math.min(100, Number(totals.selfTestPercent || 0)));
  const practicePercent = Math.round((reviewPercent + selfTestPercent) / 2);
  const sourcePercent = topicTotal > 0 ? Math.round((sourceReady / topicTotal) * 100) : 0;
  const materialPercent = topicTotal > 0 ? Math.round((materialReady / topicTotal) * 100) : 0;
  const experimentPercent = topicTotal > 0 ? Math.round((experimentReady / topicTotal) * 100) : 0;
  const score = topicTotal > 0
    ? Math.round(sourcePercent * 0.35 + materialPercent * 0.25 + experimentPercent * 0.2 + practicePercent * 0.2)
    : 0;

  let status = "not_started";
  let label = "未开始";
  let nextAction = "先保存一条有来源支撑的学习档案。";
  if (topicTotal > 0 && sourceReady < topicTotal) {
    status = "needs_evidence";
    label = "先补来源证据";
    nextAction = "先把缺证据主题里的候选来源打开核对，并标记真正有用的作者原文。";
  } else if (topicTotal > 0 && materialReady < topicTotal) {
    status = "needs_business_validation";
    label = "待业务材料验证";
    nextAction = "为已有来源证据的主题补产品、关键词、页面、广告或竞品材料。";
  } else if (topicTotal > 0 && experimentReady < topicTotal) {
    status = "needs_experiment";
    label = "待小实验验证";
    nextAction = "选择一个主题做短周期小实验，并回填真实结果。";
  } else if (topicTotal > 0 && (reviewPercent < 100 || selfTestPercent < 100)) {
    status = "in_progress";
    label = "复盘自测中";
    nextAction = "完成剩余复盘项和自测卡，确认不是只看过而是真掌握。";
  } else if (topicTotal > 0) {
    status = "ready";
    label = "可扩展新主题";
    nextAction = "选择新的亚马逊经营问题，继续建立下一条证据到验证路径。";
  }

  const stages = [
    masteryStage("source_evidence", "来源证据", sourceReady, topicTotal, sourcePercent, sourceReady < topicTotal, "只统计已采纳作者原文证据。"),
    masteryStage("business_materials", "业务材料", materialReady, topicTotal, materialPercent, sourceReady >= topicTotal && materialReady < topicTotal, "只统计用户补充的产品、页面、关键词、广告或竞品材料。"),
    masteryStage("experiments", "小实验", experimentReady, topicTotal, experimentPercent, sourceReady >= topicTotal && materialReady >= topicTotal && experimentReady < topicTotal, "只统计已回填的真实实验复盘。"),
    {
      id: "review_self_test",
      label: "复盘自测",
      done: Math.round(((totals.reviewCompleted || 0) + (totals.selfTestMastered || 0))),
      total: Math.round(((totals.reviewTotal || 0) + (totals.selfTestTotal || 0))),
      percent: practicePercent,
      status: practicePercent >= 100 ? "done" : topicTotal > 0 ? "current" : "blocked",
      detail: "复盘和自测只记录理解进度，不改变来源证据。",
    },
  ];

  return {
    title: "亚马逊学习掌握面板",
    status,
    label,
    score,
    summary: topicTotal > 0
      ? `${topicTotal} 个研究主题中，${sourceReady} 个已有来源证据，${materialReady} 个补了业务材料，${experimentReady} 个有实验复盘；当前掌握度 ${score}%。`
      : "还没有形成研究主题。先保存一条有来源支撑的回答，再开始累计掌握度。",
    nextAction,
    stages,
    topics: paths.slice(0, 5).map((path) => ({
      id: sanitizeText(path.topicId, 80),
      label: sanitizeText(path.topicLabel, 120),
      status: sanitizeText(path.status, 40),
      statusLabel: sanitizeText(path.statusLabel, 80),
      percent: Math.max(0, Math.min(100, Number(path.progress?.percent || 0))),
      nextAction: sanitizeText(path.nextAction, 180),
      currentStep: sanitizeText(path.currentStep?.label, 80),
      boundary: "主题掌握度来自学习档案进度；业务材料和实验复盘不是作者原文证据。",
    })),
    boundary: "掌握面板只整理你的学习进度、证据采纳和业务验证状态，不写入原始知识库；作者原文、系统整理、用户业务材料必须继续分开看。",
  };
}

function masteryStage(id, label, done, total, percent, current, detail) {
  return {
    id,
    label,
    done,
    total,
    percent,
    status: total <= 0 ? "blocked" : done >= total ? "done" : current ? "current" : "blocked",
    detail,
  };
}

function buildResearchMission(item) {
  const dossier = item.dossier;
  const business = item.businessVerification || buildBusinessVerificationPanel(dossier);
  const progress = item.progress || { completed: 0, total: 0, percent: 0 };
  const selfTestProgress = item.selfTestProgress || { mastered: 0, total: 0, percent: 0 };
  const sourceScope = dossier.allowedAuthors.length > 0 ? `只看 ${dossier.allowedAuthors.join("、")}` : "全部作者";
  const base = {
    id: dossier.id,
    title: dossier.title,
    question: dossier.question,
    takeaway: dossier.takeaway,
    createdAt: dossier.createdAt,
    sourceScope,
    evidenceCount: dossier.acceptedEvidence.length,
    sourceCount: dossier.sources.length,
    materialRecords: dossier.businessVerificationRecords.length,
    readyMaterialRecords: dossier.businessVerificationRecords.filter((record) => record.status === "ready").length,
    experimentResults: dossier.experimentResultRecords.length,
    positiveExperimentResults: dossier.experimentResultRecords.filter((record) => record.outcome === "positive").length,
    reviewProgress: progress,
    selfTestProgress,
  };

  if (dossier.acceptedEvidence.length === 0 || dossier.sources.length === 0) {
    return {
      ...base,
      stage: "evidence",
      stageLabel: "固化证据",
      nextAction: "先确认可用原文证据",
      reason: dossier.sources.length === 0 ? "这个档案没有来源身份，继续前先换更具体的问题补来源。" : "这个档案还没有标记有用证据，后续判断容易失去依据。",
      boundary: "先固化作者原文证据；用户材料和实验结果不能替代来源证据。",
      prompt: dossier.followUps[0] || dossier.question,
    };
  }

  if (dossier.businessVerificationRecords.length === 0) {
    return {
      ...base,
      stage: "materials",
      stageLabel: "补产品材料",
      nextAction: "粘贴真实产品数据",
      reason: "已有学习结论，但还没把你的产品、关键词、页面或广告数据放进来验证。",
      boundary: "产品材料只用于验证你的业务情况，不会写入原始知识库，也不会变成作者原文。",
      prompt: item.nextItem?.prompt || dossier.followUps[0] || dossier.question,
    };
  }

  if (dossier.experimentResultRecords.length === 0) {
    const coverage = business.coverage || {};
    if ((coverage.total || 0) > 0 && (coverage.complete || 0) < coverage.total) {
      const priority = business.validationPlan?.priorityDimension;
      return {
        ...base,
        stage: "verification",
        stageLabel: "补验证缺口",
        nextAction: priority?.label ? `补齐 ${priority.label}` : "补齐关键验证材料",
        reason: business.validationPlan?.summary || "已有产品材料，但仍有维度不完整，先补缺口再判断。",
        boundary: "验证缺口来自用户材料整理；它只能帮助判断你的产品，不能补足作者来源证据。",
        prompt: priority?.prompt || item.nextItem?.prompt || dossier.followUps[0] || dossier.question,
      };
    }
    const experiment = business.validationPlan?.experiments?.[0];
    return {
      ...base,
      stage: "experiment",
      stageLabel: "跑小实验",
      nextAction: experiment?.title || "设计 1 个小实验",
      reason: "材料已经进入验证阶段，但还没有回填实验结果。",
      boundary: "小实验结果只属于当前学习档案，不会改写作者资料或原始知识库。",
      prompt: experiment?.prompt || item.nextItem?.prompt || dossier.followUps[0] || dossier.question,
    };
  }

  if ((progress.total || 0) > (progress.completed || 0)) {
    return {
      ...base,
      stage: "review",
      stageLabel: "复盘执行",
      nextAction: item.nextItem?.label || "完成剩余推进项",
      reason: `还有 ${Math.max(0, (progress.total || 0) - (progress.completed || 0))} 个推进项未处理。`,
      boundary: "复盘状态只记录你的执行进度，不会改变原文证据。",
      prompt: item.nextItem?.prompt || dossier.followUps[0] || dossier.question,
    };
  }

  if ((selfTestProgress.total || 0) > (selfTestProgress.mastered || 0)) {
    return {
      ...base,
      stage: "self_test",
      stageLabel: "自测理解",
      nextAction: "完成自测卡",
      reason: `还有 ${Math.max(0, (selfTestProgress.total || 0) - (selfTestProgress.mastered || 0))} 张自测卡未确认掌握。`,
      boundary: "自测只用于确认你是否理解档案，不会影响资料来源。",
      prompt: dossier.followUps[0] || dossier.question,
    };
  }

  return {
    ...base,
    stage: "ready",
    stageLabel: "可继续扩展",
    nextAction: "带新问题继续追问",
    reason: "这个档案已经有证据、材料、实验和复盘记录，可以继续拓展下一轮问题。",
    boundary: "继续扩展仍会保留来源边界和作者范围。",
    prompt: dossier.followUps[0] || dossier.question,
  };
}

function buildOverviewStudyMaterials(summaries, totals) {
  if (!Array.isArray(summaries) || summaries.length === 0) return [];
  const latest = summaries.slice().sort((a, b) => String(b.dossier.createdAt).localeCompare(String(a.dossier.createdAt)))[0];
  const questions = uniqueOrdered(
    summaries.flatMap((item) => [
      item.dossier.question,
      ...item.dossier.followUps,
      item.nextItem?.prompt,
    ]),
    6,
  );
  const keyTakeaways = uniqueOrdered(summaries.map((item) => item.dossier.takeaway).filter(Boolean), 5);
  const openMissions = summaries.map(buildResearchMission).filter((mission) => mission && mission.stage !== "ready");
  const evidenceGapCount = summaries.filter((item) => item.dossier.acceptedEvidence.length === 0 || item.dossier.sources.length === 0).length;
  const evidenceNotes = uniqueOrdered(
    summaries.flatMap((item) =>
      item.dossier.acceptedEvidence.slice(0, 2).map((evidence) => {
        const source = [evidence.author, evidence.title].filter(Boolean).join("《");
        const sourceText = evidence.author && evidence.title ? `${source}》` : source || evidence.title || "来源";
        return `${sourceText}：${evidence.quote || evidence.text || ""}`;
      }),
    ),
    5,
  );
  return [
    {
      type: "brief",
      title: "本地学习摘要",
      body: `${totals.dossiers} 个档案中，${totals.businessVerificationOpen} 个还没补产品材料，${evidenceGapCount} 个需要先固化证据。最近主题：${latest?.dossier.title || latest?.dossier.question || "暂无"}。`,
      items: keyTakeaways,
    },
    evidenceNotes.length
      ? {
          type: "evidence_notes",
          title: "已采纳原文证据",
          body: "这些摘录来自你标记有用的来源，只用于回看证据边界。",
          items: evidenceNotes,
        }
      : null,
    {
      type: "faq",
      title: "下一轮关键问题",
      body: "这些问题来自已保存档案和待推进任务，可直接填入问答入口继续。",
      items: questions,
    },
    {
      type: "review_cards",
      title: "复习卡",
      body: "优先复习还没有完成自测或复盘的档案。",
      items: openMissions.slice(0, 5).map((mission) => `${mission.stageLabel}：${mission.nextAction}`),
    },
  ].filter((item) => item && (item.body || item.items.length > 0));
}

const RESEARCH_TOPIC_DEFS = [
  {
    id: "visual",
    label: "主图与视觉转化",
    keywords: ["主图", "副图", "图片", "视觉", "点击率", "ctr", "转化率", "cvr", "场景图", "对比图"],
  },
  {
    id: "listing",
    label: "Listing 与页面承接",
    keywords: ["listing", "标题", "五点", "bullet", "页面", "文案", "a+", "评价", "review", "价格"],
  },
  {
    id: "ads",
    label: "广告与投放验证",
    keywords: ["广告", "acos", "ppc", "sp", "sbv", "预算", "竞价", "campaign", "投放"],
  },
  {
    id: "keywords",
    label: "关键词与竞品意图",
    keywords: ["关键词", "搜索词", "竞品", "竞对", "asin", "类目", "排名", "keyword", "search term"],
  },
  {
    id: "selection",
    label: "选品与机会判断",
    keywords: ["选品", "新品", "机会", "值得做", "利润", "退货", "风险", "供应链", "需求"],
  },
];

function buildResearchTopicGroups(summaries = []) {
  const groups = new Map();
  for (const item of summaries) {
    const topic = detectResearchTopic(item.dossier);
    const current = groups.get(topic.id) || {
      id: topic.id,
      label: topic.label,
      dossiers: [],
      dossierCount: 0,
      evidenceCount: 0,
      candidateSourceCount: 0,
      materialRecords: 0,
      experimentResults: 0,
      openMissionCount: 0,
      keyTakeaways: [],
      keySources: [],
      nextActions: [],
      claimSupport: [],
      supportedConclusions: [],
      validationHypotheses: [],
      materialGaps: [],
      readingQueue: [],
      acceptedSourceItems: [],
      candidateSourceItems: [],
      excludedSourceItems: [],
      sourceScope: "全部作者",
      businessMaterialLines: [],
      experimentResultLines: [],
    };
    const dossier = item.dossier;
    const mission = buildResearchMission(item);
    const sourceBuckets = buildDossierTopicSources(dossier);
    const traceableAcceptedEvidence = traceableEvidenceList(dossier.acceptedEvidence);
    current.dossierCount += 1;
    current.evidenceCount += traceableAcceptedEvidence.length;
    current.candidateSourceCount += dossier.sources.length;
    current.materialRecords += dossier.businessVerificationRecords.length;
    current.experimentResults += dossier.experimentResultRecords.length;
    if (mission && mission.stage !== "ready") current.openMissionCount += 1;
    current.dossiers.push({
      id: dossier.id,
      title: dossier.title,
      question: dossier.question,
      stage: mission?.stage || "",
      stageLabel: mission?.stageLabel || "",
    });
    if (dossier.takeaway) current.keyTakeaways.push(dossier.takeaway);
    const dossierClaimSupport = buildTopicClaimSupport(dossier);
    current.claimSupport.push(...dossierClaimSupport);
    current.readingQueue.push({
      id: dossier.id,
      title: dossier.title,
      reason: mission?.stageLabel ? `${mission.stageLabel}：${mission.nextAction}` : "继续回看这个学习档案。",
      hasEvidence: traceableAcceptedEvidence.length > 0,
    });
    if (dossierClaimSupport.length > 0) {
      current.supportedConclusions.push(...dossierClaimSupport.map((item) => `${item.claim}（${item.supportLabel} ${item.evidenceCount} 条）`));
    }
    current.validationHypotheses.push(...dossier.nextActions.map((action) => `${action}（需要用你的产品数据验证）`));
    current.materialGaps.push(...topicMaterialGaps(dossier, mission));
    current.keySources.push(...traceableAcceptedEvidence.map(formatEvidenceNote));
    current.businessMaterialLines.push(...dossier.businessVerificationRecords.map((record) => formatBusinessMaterialLine(dossier, record)));
    current.experimentResultLines.push(...dossier.experimentResultRecords.map((record) => formatExperimentResultLine(dossier, record)));
    current.acceptedSourceItems.push(...sourceBuckets.accepted);
    current.candidateSourceItems.push(...sourceBuckets.candidates);
    current.excludedSourceItems.push(...sourceBuckets.excluded);
    current.nextActions.push(...dossier.nextActions, ...dossier.followUps);
    current.sourceScope = mergeTopicScope(current.sourceScope, dossier.allowedAuthors);
    groups.set(topic.id, current);
  }

  return [...groups.values()]
    .map((group) => ({
      ...group,
      dossiers: group.dossiers.slice(0, 6),
      keyTakeaways: uniqueOrdered(group.keyTakeaways, 5),
      keySources: uniqueOrdered(group.keySources, 5),
      nextActions: uniqueOrdered(group.nextActions, 6),
      claimSupport: dedupeClaimSupport(group.claimSupport, 5),
      supportedConclusions: uniqueOrdered(group.supportedConclusions, 5),
      validationHypotheses: uniqueOrdered(group.validationHypotheses, 6),
      materialGaps: uniqueOrdered(group.materialGaps, 6),
      readingQueue: group.readingQueue.slice(0, 5),
      businessMaterialLines: uniqueOrdered(group.businessMaterialLines, 6),
      experimentResultLines: uniqueOrdered(group.experimentResultLines, 6),
      evidenceStatus: buildTopicEvidenceStatus(group),
      learningPath: buildTopicLearningPath(group),
      sourcePackage: buildTopicSourcePackage(group),
    }))
    .sort((a, b) =>
      b.dossierCount - a.dossierCount
      || b.evidenceCount - a.evidenceCount
      || b.materialRecords - a.materialRecords
      || a.label.localeCompare(b.label),
    )
    .slice(0, 6);
}

function buildTopicEvidenceStatus(group = {}) {
  const evidenceCount = Number(group.evidenceCount || 0);
  const materialRecords = Number(group.materialRecords || 0);
  const experimentResults = Number(group.experimentResults || 0);
  const openMissionCount = Number(group.openMissionCount || 0);
  if (evidenceCount <= 0) {
    return {
      level: "needs_evidence",
      label: "待补证据",
      summary: "这个主题还没有已采纳原文证据，不能生成正式结论。",
    };
  }
  if (materialRecords <= 0) {
    return {
      level: "source_backed",
      label: "有来源，待业务验证",
      summary: "已有作者原文证据，但还没放入你的产品、关键词、页面或广告材料。",
    };
  }
  if (experimentResults <= 0) {
    return {
      level: "needs_experiment",
      label: "待小实验验证",
      summary: "已有来源证据和用户材料，但还没有回填小实验结果。",
    };
  }
  if (openMissionCount > 0) {
    return {
      level: "in_progress",
      label: "验证中",
      summary: "已有证据、材料和实验记录，但仍有复盘或自测任务未完成。",
    };
  }
  return {
    level: "ready",
    label: "可继续扩展",
    summary: "这个主题已有证据、材料、实验和复盘记录，可以继续拓展下一轮问题。",
  };
}

function traceableEvidenceList(items = []) {
  return (Array.isArray(items) ? items : []).filter(isTraceableEvidence);
}

function isTraceableEvidence(item = {}) {
  const sourcePath = sanitizeText(item.sourcePath, 260);
  const sourceUrl = sanitizeText(item.sourceUrl, 260);
  const title = sanitizeText(item.title, 180);
  const quote = sanitizeText(item.quote || item.text, 260);
  return !!quote && !!title && (!!sourcePath || !!sourceUrl);
}

function buildTopicLearningPath(group = {}) {
  const evidenceCount = Number(group.evidenceCount || 0);
  const materialRecords = Number(group.materialRecords || 0);
  const experimentResults = Number(group.experimentResults || 0);
  const openMissionCount = Number(group.openMissionCount || 0);
  const evidenceDone = evidenceCount > 0;
  const materialDone = materialRecords > 0;
  const experimentDone = experimentResults > 0;
  return [
    {
      id: "read_sources",
      label: "先读来源证据",
      status: evidenceDone ? "done" : "current",
      reason: evidenceDone ? `已有 ${evidenceCount} 条已采纳原文证据。` : "先回到相关回答，标记真正有用的来源摘录。",
    },
    {
      id: "add_materials",
      label: "再补产品材料",
      status: !evidenceDone ? "blocked" : materialDone ? "done" : "current",
      reason: materialDone ? `已有 ${materialRecords} 条用户业务材料。` : "需要产品、关键词、页面、广告或竞品信息来验证是否适用于你的业务。",
    },
    {
      id: "run_experiment",
      label: "再做小实验",
      status: !evidenceDone || !materialDone ? "blocked" : experimentDone ? "done" : "current",
      reason: experimentDone ? `已有 ${experimentResults} 条实验复盘。` : "用小样本、短周期实验验证点击率、转化率或广告表现。",
    },
    {
      id: "review_understanding",
      label: "最后复盘自测",
      status: !evidenceDone || !materialDone || !experimentDone ? "blocked" : openMissionCount > 0 ? "current" : "done",
      reason: !evidenceDone || !materialDone || !experimentDone
        ? "完成前面的证据、材料和实验步骤后，再做复盘自测。"
        : openMissionCount > 0
          ? `还有 ${openMissionCount} 个推进项未完成。`
          : "复盘和自测已进入可继续扩展状态。",
    },
  ];
}

function buildDossierTopicSources(dossier = {}) {
  const accepted = traceableEvidenceList(dossier.acceptedEvidence)
    .map((item, index) => ({
        id: sanitizeText(item.id || `accepted:${index}`, 80),
        kind: "accepted",
        dossierId: sanitizeText(dossier.id, 80),
        dossierTitle: sanitizeText(dossier.title || dossier.question || "学习档案", 140),
        author: sanitizeText(item.author, 80),
        title: sanitizeText(item.title, 180),
        date: sanitizeText(item.date, 32),
        quote: sanitizeText(item.quote || item.text, 260),
        sourceUrl: sanitizeText(item.sourceUrl, 260),
        sourcePath: sanitizeText(item.sourcePath, 260),
        sourceKey: sanitizeText(item.sourceKey || sourceKeyForSource(item), 360),
      })).filter((item) => item.title || item.quote);
  const acceptedKeys = new Set(accepted.flatMap((item) => [item.sourceKey, item.sourcePath, item.sourceUrl].filter(Boolean)));
  const rejectedKeys = new Set((Array.isArray(dossier.rejectedEvidence) ? dossier.rejectedEvidence : []).flatMap(evidenceIdentityKeys));
  const excludedKeys = new Set((Array.isArray(dossier.excludedSources) ? dossier.excludedSources : []).map((item) => item.key).filter(Boolean));
  const candidates = (Array.isArray(dossier.sources) ? dossier.sources : [])
    .filter((source) => {
      const keys = sourceIdentityKeys(source);
      return !keys.some((key) => acceptedKeys.has(key) || rejectedKeys.has(key) || excludedKeys.has(key));
    })
    .map((source, index) => ({
      id: sanitizeText(`candidate:${sourceKeyForSource(source) || index}`, 100),
      kind: "candidate",
      dossierId: sanitizeText(dossier.id, 80),
      dossierTitle: sanitizeText(dossier.title || dossier.question || "学习档案", 140),
      author: sanitizeText(source.author, 80),
      title: sanitizeText(source.title, 180),
      date: sanitizeText(source.date, 32),
      excerpt: sanitizeText(source.excerpt, 260),
      sourceUrl: sanitizeText(source.sourceUrl, 260),
      sourcePath: sanitizeText(source.sourcePath, 260),
      sourceKey: sanitizeText(sourceKeyForSource(source), 360),
    }))
    .filter((item) => item.title || item.excerpt);
  const excluded = (Array.isArray(dossier.excludedSources) ? dossier.excludedSources : [])
    .map((item, index) => ({
      id: sanitizeText(`excluded:${item.key || index}`, 100),
      kind: "excluded",
      dossierId: sanitizeText(dossier.id, 80),
      dossierTitle: sanitizeText(dossier.title || dossier.question || "学习档案", 140),
      label: sanitizeText(item.label || item.key, 220),
      sourceKey: sanitizeText(item.key, 360),
    }))
    .filter((item) => item.label || item.sourceKey);
  return { accepted, candidates, excluded };
}

function buildTopicSourcePackage(group = {}) {
  const accepted = dedupeTopicSources(group.acceptedSourceItems, 6);
  const candidates = dedupeTopicSources(group.candidateSourceItems, 6);
  const excluded = dedupeTopicSources(group.excludedSourceItems, 6);
  const acceptedSourceKeys = accepted.map((item) => item.sourceKey).filter(Boolean).slice(0, 50);
  const selectedSources = accepted.map(topicSourceForControl).filter(Boolean).slice(0, 12);
  const authors = uniqueOrdered(
    [...accepted, ...candidates].map((item) => item.author).filter(Boolean),
    5,
  );
  const selectedDossierId =
    accepted[0]?.dossierId ||
    candidates[0]?.dossierId ||
    excluded[0]?.dossierId ||
    (Array.isArray(group.dossiers) ? group.dossiers[0]?.id : "");
  const acceptedTitles = accepted.slice(0, 3).map((item) => item.title).filter(Boolean).join("、");
  const candidateTitles = candidates.slice(0, 2).map((item) => item.title).filter(Boolean).join("、");
  const nextPrompt = accepted.length > 0
    ? `请基于「${group.label || "这个主题"}」主题的已采纳来源继续学习，优先使用这些来源：${acceptedTitles || "已采纳原文证据"}。请区分作者原文、系统整理和我的业务验证。`
    : candidates.length > 0
      ? `请围绕「${group.label || "这个主题"}」主题重新核对这些候选来源：${candidateTitles || "候选来源"}。先判断哪些能采纳为原文证据，再生成学习结论。`
      : `请帮我为「${group.label || "这个主题"}」主题重新检索更具体的作者来源；没有来源前不要生成正式结论。`;

  return {
    title: `${sanitizeText(group.label || "研究主题", 120)}来源包`,
    summary: accepted.length > 0
      ? `已有 ${accepted.length} 条已采纳证据、${candidates.length} 条候选来源、${excluded.length} 条已排除来源。`
      : candidates.length > 0
        ? `还没有已采纳证据，先从 ${candidates.length} 条候选来源里确认可用原文。`
        : `这个主题还缺可定位来源；继续学习前先补来源。`,
    status: accepted.length > 0 ? "source_backed" : candidates.length > 0 ? "needs_adoption" : "needs_source",
    accepted,
    candidates,
    excluded,
    authors,
    selectedDossierId: sanitizeText(selectedDossierId, 80),
    nextPrompt: sanitizeText(nextPrompt, 520),
    sourceControls: {
      excludedSourceKeys: excluded.map((item) => item.sourceKey).filter(Boolean).slice(0, 30),
      allowedAuthors: group.sourceScope && group.sourceScope !== "全部作者"
        ? normalizeAuthorNames(group.sourceScope.replace(/^只看\s*/, "").split("、"))
        : [],
      allowedSourceKeys: acceptedSourceKeys,
      selectedSources,
    },
    boundary: "来源包只整理已保存学习档案里的作者来源；候选来源未被采纳前不能当成结论依据，用户业务材料和实验复盘也不会进入来源包。",
  };
}

function topicSourceForControl(item = {}) {
  const sourceKey = sanitizeText(item.sourceKey, 360);
  return {
    author: sanitizeText(item.author, 80),
    title: sanitizeText(item.title || item.label, 180),
    date: sanitizeText(item.date, 32),
    excerpt: sanitizeText(item.quote || item.excerpt, 320),
    sourceUrl: sanitizeText(item.sourceUrl, 260),
    sourcePath: sanitizeText(item.sourcePath, 260),
    sourceKey,
  };
}

function dedupeTopicSources(items = [], limit = 6) {
  const seen = new Set();
  const rows = [];
  for (const item of Array.isArray(items) ? items : []) {
    const key = sanitizeText(item?.sourceKey || item?.sourcePath || item?.sourceUrl || item?.label || item?.title, 360);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    rows.push({
      id: sanitizeText(item?.id || key, 120),
      kind: sanitizeText(item?.kind || "source", 40),
      dossierId: sanitizeText(item?.dossierId, 80),
      dossierTitle: sanitizeText(item?.dossierTitle, 140),
      author: sanitizeText(item?.author, 80),
      title: sanitizeText(item?.title, 180),
      date: sanitizeText(item?.date, 32),
      quote: sanitizeText(item?.quote, 260),
      excerpt: sanitizeText(item?.excerpt, 260),
      label: sanitizeText(item?.label, 220),
      sourceUrl: sanitizeText(item?.sourceUrl, 260),
      sourcePath: sanitizeText(item?.sourcePath, 260),
      sourceKey: key,
    });
    if (rows.length >= limit) break;
  }
  return rows;
}

function buildOverviewLearningPaths(topicGroups = [], researchMissions = []) {
  const missionByDossierId = new Map(
    (Array.isArray(researchMissions) ? researchMissions : [])
      .filter((mission) => mission && mission.id)
      .map((mission) => [mission.id, mission]),
  );
  return (Array.isArray(topicGroups) ? topicGroups : [])
    .map((group) => {
      const steps = (Array.isArray(group.learningPath) && group.learningPath.length
        ? group.learningPath
        : buildTopicLearningPath(group)
      ).map((step) => ({
        id: sanitizeText(step.id || "", 80),
        label: sanitizeText(step.label || "", 80),
        status: sanitizeText(step.status || "todo", 32),
        reason: sanitizeText(step.reason || "", 220),
      }));
      const done = steps.filter((step) => step.status === "done").length;
      const total = steps.length;
      const currentStep = steps.find((step) => step.status === "current")
        || steps.find((step) => step.status === "blocked")
        || steps[steps.length - 1]
        || null;
      const relatedDossiers = (Array.isArray(group.dossiers) ? group.dossiers : [])
        .slice(0, 4)
        .map((dossier) => ({
          id: sanitizeText(dossier.id || "", 120),
          title: sanitizeText(dossier.title || dossier.question || "学习档案", 140),
          question: sanitizeText(dossier.question || "", 180),
          stage: sanitizeText(dossier.stage || "", 40),
          stageLabel: sanitizeText(dossier.stageLabel || "", 80),
        }));
      const relatedMissions = relatedDossiers
        .map((dossier) => missionByDossierId.get(dossier.id))
        .filter(Boolean);
      const activeMission = relatedMissions.find((mission) => mission.stage !== "ready") || relatedMissions[0] || null;
      const status = sanitizeText(group.evidenceStatus?.level || "unknown", 40);
      const needsSourceEvidence = status === "needs_evidence";
      const needsProductMaterials = status === "source_backed";
      const needsExperimentReview = status === "needs_experiment";
      const materialTemplate = needsProductMaterials ? buildLearningPathMaterialTemplate(group) : null;
      const experimentTemplate = needsExperimentReview ? buildLearningPathExperimentTemplate(group) : null;
      const nextAction = needsSourceEvidence && currentStep?.reason
        ? currentStep.reason
        : needsProductMaterials
          ? "补产品材料，验证作者证据是否适用于你的业务。"
          : needsExperimentReview
            ? `看验证方案：${activeMission?.nextAction || "选择 1 个小实验"}，执行后再回填真实结果。`
            : activeMission?.nextAction || currentStep?.reason || group.nextActions?.[0] || "继续补充学习档案。";
      return {
        topicId: sanitizeText(group.id || "", 80),
        topicLabel: sanitizeText(group.label || "研究主题", 120),
        status,
        statusLabel: sanitizeText(group.evidenceStatus?.label || topicPathStatusLabelForData(currentStep?.status), 80),
        summary: sanitizeText(group.evidenceStatus?.summary || currentStep?.reason || "", 240),
        sourceScope: sanitizeText(group.sourceScope || "全部作者", 120),
        candidateSourceCount: Number(group.candidateSourceCount || 0),
        progress: {
          done,
          total,
          percent: total > 0 ? Math.round((done / total) * 100) : 0,
        },
        metrics: {
          dossiers: Number(group.dossierCount || 0),
          evidence: Number(group.evidenceCount || 0),
          candidateSources: Number(group.candidateSourceCount || 0),
          materials: Number(group.materialRecords || 0),
          experiments: Number(group.experimentResults || 0),
          openMissions: Number(group.openMissionCount || 0),
        },
        currentStep,
        steps,
        nextAction: sanitizeText(nextAction, 220),
        nextPrompt: sanitizeText(activeMission?.prompt || group.nextActions?.[0] || "", 260),
        materialTemplate,
        experimentTemplate,
        relatedDossiers,
        boundary: "主题学习路径只整理已保存学习档案，不写入原始知识库；用户业务材料和实验复盘不等同于作者原文证据。",
      };
    })
    .sort((a, b) => learningPathPriority(a.status) - learningPathPriority(b.status) || (b.metrics.dossiers || 0) - (a.metrics.dossiers || 0))
    .slice(0, 6);
}

function learningPathPriority(status) {
  if (status === "needs_evidence") return 0;
  if (status === "source_backed") return 1;
  if (status === "needs_experiment") return 2;
  if (status === "in_progress") return 3;
  if (status === "ready") return 4;
  return 5;
}

function topicPathStatusLabelForData(status) {
  if (status === "done") return "已完成";
  if (status === "current") return "当前";
  if (status === "blocked") return "等待";
  return "待处理";
}

function buildLearningPathMaterialTemplate(group = {}) {
  const topicLabel = sanitizeText(group.label || "研究主题", 80);
  const text = [
    "产品/ASIN：",
    "主图现状：",
    "核心关键词：",
    "CTR：",
    "CVR：",
    "竞品/对标：",
    "广告/ACOS：",
  ].join("\n");
  return {
    title: "补产品材料",
    topicLabel,
    text,
    boundary: "这些是你的产品材料，不是作者原文证据；只用于验证作者方法是否适用于你的业务。",
  };
}

function buildLearningPathExperimentTemplate(group = {}) {
  const topicLabel = sanitizeText(group.label || "研究主题", 80);
  const text = [
    "实验名称：",
    "改动项：",
    "周期：",
    "CTR 前/后：",
    "CVR 前/后：",
    "ACOS 前/后：",
    "结论：",
  ].join("\n");
  return {
    title: "实验复盘",
    topicLabel,
    text,
    boundary: "这是你的实验复盘，不是作者原文证据；保存材料不代表已验证，必须回填真实结果再判断。",
  };
}

function buildTopicClaimSupport(dossier) {
  if (!dossier || dossier.acceptedEvidence.length === 0) return [];
  const traceableEvidence = traceableEvidenceList(dossier.acceptedEvidence);
  if (traceableEvidence.length === 0) return [];
  const claims = uniqueOrdered([
    dossier.takeaway,
    ...dossier.conclusions,
  ], 4);
  return claims
    .map((claim, index) => {
      if (isBusinessSpecificClaim(claim)) return null;
      const evidence = matchedEvidenceForClaim(claim, traceableEvidence);
      if (evidence.length === 0) return null;
      return {
        id: sanitizeText(`${dossier.id}:claim:${index}`, 120),
        claim,
        supportLevel: "source_supported",
        supportLabel: "已采纳原文支撑",
        evidenceCount: evidence.length,
        dossierId: dossier.id,
        dossierTitle: dossier.title,
        boundary: "这些证据只能证明这条结论有原文依据；是否适用于你的产品，仍要看业务材料和实验。",
        evidence,
      };
    })
    .filter(Boolean);
}

function matchedEvidenceForClaim(claim, acceptedEvidence = []) {
  return acceptedEvidence
    .map((item, index) => ({ score: claimEvidenceScore(claim, item), item, index }))
    .filter((entry) => entry.score > 0)
    .sort((a, b) => b.score - a.score)
    .slice(0, 3)
    .map(({ item, index }) => ({
      id: sanitizeText(item.id || `evidence:${index}`, 80),
      label: sanitizeText(`${item.author || "未知作者"}《${item.title || "未命名来源"}》`, 180),
      quote: sanitizeText(item.quote || item.text, 260),
      author: sanitizeText(item.author, 80),
      title: sanitizeText(item.title, 180),
      date: sanitizeText(item.date, 32),
      sourceKey: sanitizeText(item.sourceKey || sourceKeyForSource(item), 260),
      sourceUrl: sanitizeText(item.sourceUrl, 260),
      sourcePath: sanitizeText(item.sourcePath, 260),
    }))
    .filter((item) => item.quote || item.label);
}

function claimEvidenceScore(claim, evidence) {
  const claimText = normalizeMatchText(claim);
  const evidenceText = normalizeMatchText(`${evidence?.title || ""} ${evidence?.quote || ""} ${evidence?.text || ""}`);
  if (!claimText || !evidenceText) return 0;
  if (hasUnsupportedClaimScope(claimText, evidenceText)) return 0;
  let score = 0;
  for (const aliases of TOPIC_CLAIM_ALIASES) {
    const claimHas = aliases.some((term) => claimText.includes(term));
    const evidenceHas = aliases.some((term) => evidenceText.includes(term));
    if (claimHas && evidenceHas) score += 3;
  }
  for (const term of claimMatchTerms(claimText)) {
    if (evidenceText.includes(term)) score += term.length >= 4 ? 2 : 1;
  }
  return score;
}

function hasUnsupportedClaimScope(claimText, evidenceText) {
  const strongInferenceTerms = ["只要", "一定", "必然", "肯定", "保证", "不需要", "不用", "无需"];
  if (strongInferenceTerms.some((term) => claimText.includes(term) && !evidenceText.includes(term))) return true;
  const scopedAliases = [
    ["转化率", "cvr", "转化"],
    ["价格", "售价"],
    ["评价", "review"],
    ["页面", "文案", "listing", "五点", "a+"],
    ["广告", "acos", "ppc", "sp", "sbv"],
  ];
  return scopedAliases.some((aliases) =>
    aliases.some((term) => claimText.includes(term)) &&
    !aliases.some((term) => evidenceText.includes(term))
  );
}

function isBusinessSpecificClaim(claim) {
  const text = String(claim || "").toLowerCase();
  if (!text.trim()) return false;
  const businessSignals = [
    /(^|[^\u4e00-\u9fff])ctr([^\u4e00-\u9fff]|$)/i,
    /(^|[^\u4e00-\u9fff])cvr([^\u4e00-\u9fff]|$)/i,
    /(^|[^\u4e00-\u9fff])acos([^\u4e00-\u9fff]|$)/i,
    /asin/i,
    /\d+(?:\.\d+)?\s*%/,
    /你的|我的|当前|这个产品|该产品|白底图|竞品|广告数据|session/i,
  ];
  return businessSignals.some((pattern) => pattern.test(text));
}

const TOPIC_CLAIM_ALIASES = [
  ["主图", "首图", "图片", "视觉"],
  ["点击率", "ctr", "点击"],
  ["转化率", "cvr", "转化"],
  ["副图", "场景图", "对比图"],
  ["listing", "页面", "标题", "五点", "a+"],
  ["广告", "ppc", "sp", "sbv", "acos", "投放"],
  ["关键词", "搜索词", "keyword", "searchterm"],
  ["选品", "新品", "机会", "利润", "需求"],
];

function claimMatchTerms(text) {
  const terms = new Set();
  for (const match of text.matchAll(/[a-z0-9]{3,}|[\u4e00-\u9fff]{2,6}/gi)) {
    const value = match[0].toLowerCase();
    if (value.length >= 2) terms.add(value);
  }
  return [...terms].slice(0, 20);
}

function normalizeMatchText(value) {
  return String(value || "")
    .toLowerCase()
    .replace(/\s+/g, "")
    .replace(/[【】\[\]（）()《》「」“”"'`，。；：、,.!?！？]/g, "");
}

function dedupeClaimSupport(items = [], limit = 5) {
  const seen = new Set();
  const result = [];
  for (const item of items) {
    const claim = sanitizeText(item?.claim, 260);
    if (!claim || seen.has(claim)) continue;
    seen.add(claim);
    result.push({
      id: sanitizeText(item?.id, 120),
      claim,
      supportLevel: sanitizeText(item?.supportLevel || "source_supported", 40),
      supportLabel: sanitizeText(item?.supportLabel || "已采纳原文支撑", 80),
      evidenceCount: Number(item?.evidenceCount || 0),
      dossierId: sanitizeText(item?.dossierId, 80),
      dossierTitle: sanitizeText(item?.dossierTitle, 120),
      boundary: sanitizeText(item?.boundary, 220),
      evidence: Array.isArray(item?.evidence)
        ? item.evidence.map((evidence) => ({
            id: sanitizeText(evidence?.id, 80),
            label: sanitizeText(evidence?.label, 180),
            quote: sanitizeText(evidence?.quote, 260),
            author: sanitizeText(evidence?.author, 80),
            title: sanitizeText(evidence?.title, 180),
            date: sanitizeText(evidence?.date, 32),
            sourceKey: sanitizeText(evidence?.sourceKey || sourceKeyForSource(evidence || {}), 260),
            sourceUrl: sanitizeText(evidence?.sourceUrl, 260),
            sourcePath: sanitizeText(evidence?.sourcePath, 260),
          })).filter((evidence) => evidence.quote || evidence.label).slice(0, 3)
        : [],
    });
    if (result.length >= limit) break;
  }
  return result;
}

function topicMaterialGaps(dossier, mission) {
  const title = dossier.title || dossier.question || "这个档案";
  const gaps = [];
  if (dossier.acceptedEvidence.length === 0) gaps.push(`${title}：先采纳可定位的原文证据。`);
  if (dossier.businessVerificationRecords.length === 0) gaps.push(`${title}：补真实产品、关键词、页面、广告或竞品材料。`);
  if (dossier.businessVerificationRecords.length > 0 && dossier.experimentResultRecords.length === 0) gaps.push(`${title}：补一次小实验或复盘结果。`);
  gaps.push(...dossier.missingInputs.map((item) => `${title}：${item}`));
  if (mission && mission.stage !== "ready" && mission.reason) gaps.push(`${title}：${mission.reason}`);
  return gaps;
}

function formatBusinessMaterialLine(dossier = {}, record = {}) {
  const title = dossier.title || dossier.question || "学习档案";
  const text = record.rawText || record.summary || (Array.isArray(record.sections)
    ? record.sections.flatMap((section) => section.items || []).join("；")
    : "");
  return sanitizeText(`${title}：${text}`, 420);
}

function formatExperimentResultLine(dossier = {}, record = {}) {
  const title = dossier.title || dossier.question || "学习档案";
  const text = [record.summary, record.rawText, record.nextAction].filter(Boolean).join("；");
  return sanitizeText(`${title}：${text}`, 420);
}

function buildOverviewLearningProducts(summaries = [], totals = {}, topicGroups = []) {
  if (!Array.isArray(summaries) || summaries.length === 0) return [];
  const primaryTopic = topicGroups[0] || buildResearchTopicGroups(summaries)[0] || null;
  const allQuestions = uniqueOrdered(summaries.flatMap((item) => [
    item.dossier.question,
    ...item.dossier.followUps,
    item.nextItem?.prompt,
  ]), 8);
  const allSelfTestCards = summaries
    .flatMap((item) => (item.selfTest?.items || []).map((card) => ({
      front: card.question,
      back: card.answer,
      source: card.explanation,
    })))
    .filter((card) => card.front && card.back)
    .slice(0, 8);
  const allExperimentRecords = summaries
    .flatMap((item) => item.dossier.experimentResultRecords.map((record) => ({
      dossierId: item.dossier.id,
      title: item.dossier.title,
      summary: record.summary,
      nextAction: record.nextAction,
      outcome: experimentOutcomeLabel(record.outcome),
    })))
    .slice(0, 6);

  const products = [];
  if (primaryTopic && primaryTopic.evidenceCount > 0) {
    products.push(buildTopicStudyGuideProduct(primaryTopic));
    const evidenceReport = buildTopicEvidenceReportProduct(primaryTopic);
    if (evidenceReport) products.push(evidenceReport);
  } else if (primaryTopic) {
    const sourcePackage = primaryTopic.sourcePackage || {};
    const candidateSourceCount = Array.isArray(sourcePackage.candidates)
      ? sourcePackage.candidates.length
      : Number(primaryTopic.candidateSourceCount || 0);
    products.push({
      type: "evidence_needed",
      title: `${primaryTopic.label}待补证据`,
      topicId: primaryTopic.id,
      topicLabel: primaryTopic.label,
      selectedDossierId: sanitizeText(sourcePackage.selectedDossierId, 80),
      candidateSourceCount,
      sourcePackageStatus: sanitizeText(sourcePackage.status, 40),
      nextPrompt: sanitizeText(sourcePackage.nextPrompt, 520),
      sourceSummary: sanitizeText(sourcePackage.summary, 260),
      body: `这个主题已有 ${primaryTopic.dossierCount} 个学习档案，但还没有已采纳原文证据。`,
      boundary: "缺少已采纳证据时不生成正式学习指南；候选来源必须先由你确认有用，才会进入讲义。",
      actions: [
        candidateSourceCount > 0
          ? `打开来源档案，从 ${candidateSourceCount} 条候选来源里确认真正有用的作者原文。`
          : "换一个更具体的问题重新检索本地资料，先补可定位作者来源。",
        "换一个更具体的问题重新检索本地资料。",
        "补充产品材料后，再用资料证据重新判断。",
      ],
    });
  }

  products.push({
    type: "faq_pack",
    title: "关键问题包",
    body: "这些问题来自已保存档案和待推进任务，适合继续带着上下文追问。",
    boundary: "问题包只代表当前学习路径，不代表资料已经完整覆盖。",
    questions: allQuestions.slice(0, 6).map((question) => ({
      question,
      answerHint: faqAnswerHint(question, topicGroups),
      prompt: question,
    })),
  });

  if (allSelfTestCards.length > 0) {
    products.push({
      type: "flashcards",
      title: "复习卡组",
      body: `从当前学习档案生成 ${allSelfTestCards.length} 张复习卡，用来确认你是否真正理解结论和证据边界。`,
      boundary: "复习卡答案来自已保存档案，不新增资料证据。",
      cards: allSelfTestCards,
    });
  }

  products.push({
    type: "research_brief",
    title: "本地研究简报",
    body: `${totals.dossiers || summaries.length} 个档案，${totals.acceptedEvidence || 0} 条采纳证据，${totals.businessVerificationRecords || 0} 条业务材料，${totals.experimentResults || 0} 条实验复盘。`,
    boundary: "简报是阶段性研究状态，不是最终结论；缺证据、缺材料、缺实验时会保持保守。",
    sections: [
      {
        title: "当前覆盖主题",
        items: topicGroups.length ? topicGroups.slice(0, 5).map((group) => `${group.label}：${group.dossierCount} 个档案，证据 ${group.evidenceCount} 条`) : ["还没有形成稳定主题。"],
      },
      {
        title: "主要缺口",
        items: buildOverviewGapLines(totals, topicGroups),
      },
    ],
  });

  if (allExperimentRecords.length > 0) {
    products.push({
      type: "experiment_digest",
      title: "实验复盘摘要",
      body: "把用户回填的小实验结果集中起来，避免只凭单次问答下结论。",
      boundary: "实验复盘只属于你的业务验证，不会变成作者资料。",
      records: allExperimentRecords,
    });
  }

  return products.filter((item) => item && item.title).slice(0, 6);
}

function buildTopicStudyGuideProduct(topic = {}) {
  const sourcePackage = topic.sourcePackage || {};
  const sourceBackedClaims = (Array.isArray(topic.claimSupport) ? topic.claimSupport : [])
    .filter((item) => item?.evidence?.length > 0)
    .slice(0, 5)
    .map((item) => ({
      id: sanitizeText(item.id, 120),
      claim: sanitizeText(item.claim, 260),
      supportLabel: sanitizeText(item.supportLabel || "已采纳原文支撑", 80),
      evidenceCount: Number(item.evidenceCount || item.evidence.length || 0),
      dossierId: sanitizeText(item.dossierId, 80),
      dossierTitle: sanitizeText(item.dossierTitle, 120),
      boundary: sanitizeText(item.boundary || "这条结论只绑定已采纳原文证据；是否适用于你的产品仍要验证。", 260),
      evidence: item.evidence.slice(0, 3).map((evidence) => ({
        label: sanitizeText(evidence.label, 180),
        quote: sanitizeText(evidence.quote, 260),
        author: sanitizeText(evidence.author, 80),
        title: sanitizeText(evidence.title, 180),
        date: sanitizeText(evidence.date, 32),
        dossierId: sanitizeText(evidence.dossierId || item.dossierId, 80),
        sourceKey: sanitizeText(evidence.sourceKey || sourceKeyForSource(evidence), 260),
        sourcePath: sanitizeText(evidence.sourcePath, 260),
        sourceUrl: sanitizeText(evidence.sourceUrl, 260),
      })),
    }));
  const acceptedSources = Array.isArray(sourcePackage.accepted) ? sourcePackage.accepted : [];
  const authorPerspectives = uniqueOrdered(acceptedSources.map((item) => item.author).filter(Boolean), 5)
    .map((author) => {
      const rows = acceptedSources.filter((item) => item.author === author);
      return {
        author,
        sourceCount: rows.length,
        evidenceCount: sourceBackedClaims.reduce(
          (count, claim) => count + claim.evidence.filter((evidence) => evidence.author === author).length,
          0,
        ),
        summary: sanitizeText(`${author}在这个主题下提供了 ${rows.length} 条已采纳来源，优先回看这些摘录再做业务验证。`, 220),
      };
    });
  const executionChecklist = buildTopicStudyExecutionChecklist(topic);
  const reviewQuestions = buildTopicStudyReviewQuestions(topic, sourceBackedClaims);
  const sourceControls = sourcePackage.sourceControls || {};
  const product = {
    type: "study_guide",
    title: `${sanitizeText(topic.label || "研究主题", 120)}学习指南`,
    topicId: sanitizeText(topic.id, 80),
    topicLabel: sanitizeText(topic.label, 120),
    body: `基于 ${Number(topic.dossierCount || 0)} 个学习档案、${Number(topic.evidenceCount || 0)} 条已采纳证据和 ${Number(topic.materialRecords || 0)} 条用户业务材料生成。`,
    boundary: "这是本地学习指南：只有已采纳原文证据可以支撑结论；用户业务材料和实验复盘只用于验证，不写入原始知识库。",
    sourceScope: sanitizeText(topic.sourceScope || "全部作者", 120),
    sourceBackedClaims,
    authorPerspectives,
    executionChecklist,
    reviewQuestions,
    sourceControls: {
      excludedSourceKeys: Array.isArray(sourceControls.excludedSourceKeys) ? sourceControls.excludedSourceKeys.slice(0, 30) : [],
      allowedAuthors: Array.isArray(sourceControls.allowedAuthors) ? sourceControls.allowedAuthors.slice(0, 20) : [],
      allowedSourceKeys: Array.isArray(sourceControls.allowedSourceKeys) ? sourceControls.allowedSourceKeys.slice(0, 50) : [],
      selectedSources: Array.isArray(sourceControls.selectedSources) ? sourceControls.selectedSources.slice(0, 12) : [],
    },
    nextPrompt: sourcePackage.nextPrompt || topic.nextActions?.[0] || "",
    sections: [
      {
        title: "核心结论",
        items: topic.supportedConclusions?.length ? topic.supportedConclusions : ["先保存更多有来源的回答，再生成更可靠的结论。"],
      },
      {
        title: "证据入口",
        items: topic.keySources?.length ? topic.keySources : ["这个主题还缺少已采纳原文证据，先回到问答里标记有用证据。"],
      },
      {
        title: "待验证假设",
        items: topic.validationHypotheses?.length ? topic.validationHypotheses : ["围绕这个主题继续追问，并补充具体产品材料。"],
      },
    ],
  };
  return {
    ...product,
    exportKind: "markdown_handout",
    downloadFilename: studyHandoutFilename(product.topicLabel || product.title || "学习讲义"),
    handoutMarkdown: buildTopicStudyHandoutMarkdown(product, topic),
  };
}

function buildTopicStudyHandoutMarkdown(product = {}, topic = {}) {
  const title = markdownText(product.topicLabel || topic.label || "研究主题", 120);
  const sourceClaims = Array.isArray(product.sourceBackedClaims) ? product.sourceBackedClaims : [];
  const acceptedSources = Array.isArray(topic.sourcePackage?.accepted) ? topic.sourcePackage.accepted : [];
  const candidateSources = Array.isArray(topic.sourcePackage?.candidates) ? topic.sourcePackage.candidates : [];
  const materialLines = Array.isArray(topic.businessMaterialLines) ? topic.businessMaterialLines : [];
  const experimentLines = Array.isArray(topic.experimentResultLines) ? topic.experimentResultLines : [];
  const checklist = Array.isArray(product.executionChecklist) ? product.executionChecklist : [];
  const reviewQuestions = Array.isArray(product.reviewQuestions) ? product.reviewQuestions : [];
  const sections = Array.isArray(product.sections) ? product.sections : [];
  const lines = [
    `# ${title}学习讲义`,
    "",
    "来源边界：这份讲义由本地学习档案生成。只有“作者原文证据”区里的原文片段可以作为来源支撑；系统整理、用户业务材料、实验复盘和候选来源都不能替代作者原文。",
    "",
    "## 作者原文证据",
  ];

  if (sourceClaims.length === 0) {
    lines.push("", "暂无已采纳作者原文证据。没有原文证据时，不生成正式学习结论。");
  } else {
    sourceClaims.forEach((claim, claimIndex) => {
      lines.push("", `### ${claimIndex + 1}. ${markdownText(claim.claim || "来源支撑结论", 260)}`);
      lines.push(`- 支撑状态：${markdownText(claim.supportLabel || "已采纳原文支撑", 120)}；证据 ${Number(claim.evidenceCount || 0)} 条。`);
      (Array.isArray(claim.evidence) ? claim.evidence : []).slice(0, 3).forEach((evidence, evidenceIndex) => {
        const sourceLine = [
          evidence.author,
          evidence.date,
          evidence.title,
        ].map((item) => markdownText(item, 180)).filter(Boolean).join(" · ");
        const locator = markdownText(evidence.sourcePath || evidence.sourceUrl || evidence.sourceKey || "来源片段不可用", 320);
        lines.push(`- 原文片段 ${evidenceIndex + 1}：“${markdownText(evidence.quote || "", 360)}”`);
        if (sourceLine) lines.push(`  - 来源：${sourceLine}`);
        lines.push(`  - 定位：${locator}`);
      });
      if (claim.boundary) lines.push(`- 边界：${markdownText(claim.boundary, 280)}`);
    });
  }

  lines.push("", "## 系统整理");
  const conclusionSection = sections.find((section) => section.title === "核心结论") || {};
  const conclusions = Array.isArray(conclusionSection.items) ? conclusionSection.items : [];
  if (conclusions.length) {
    lines.push("", "### 核心结论");
    conclusions.slice(0, 5).forEach((item) => lines.push(`- ${markdownText(item, 260)}`));
  }
  if (checklist.length) {
    lines.push("", "### 可执行清单");
    checklist.slice(0, 6).forEach((item) => {
      const mark = item.done ? "已完成" : "待处理";
      lines.push(`- ${mark}：${markdownText(item.label || "", 160)}${item.reason ? `。${markdownText(item.reason, 220)}` : ""}`);
    });
  }
  if (reviewQuestions.length) {
    lines.push("", "### 复习追问");
    reviewQuestions.slice(0, 6).forEach((item) => lines.push(`- ${markdownText(item.question || "", 260)}`));
  }

  lines.push("", "## 用户业务材料与实验");
  if (materialLines.length === 0 && experimentLines.length === 0) {
    lines.push("- 暂无用户业务材料或实验复盘。这些不是作者原文证据，补充后也只用于验证适用性。");
  } else {
    materialLines.slice(0, 6).forEach((item) => lines.push(`- 业务材料：${markdownText(item, 320)}（这些不是作者原文证据）`));
    experimentLines.slice(0, 6).forEach((item) => lines.push(`- 实验复盘：${markdownText(item, 320)}（这些不是作者原文证据）`));
  }

  lines.push("", "## 候选来源与限制");
  if (candidateSources.length) {
    lines.push("- 候选来源仅用于下一轮核对，未被采纳前不能当成结论依据。");
    candidateSources.slice(0, 6).forEach((source) => {
      const locator = source.sourcePath || source.sourceUrl || source.sourceKey || "来源片段不可用";
      lines.push(`- 候选：${markdownText([source.author, source.title].filter(Boolean).join(" · ") || source.label || "未命名来源", 220)}；定位：${markdownText(locator, 300)}`);
    });
  } else {
    lines.push("- 暂无额外候选来源。所有正式证据仍以上方“作者原文证据”区为准。");
  }
  if (acceptedSources.length) {
    lines.push(`- 已采纳来源数量：${acceptedSources.length}。复制或导出讲义不会自动采纳新证据，也不会写入原始知识库。`);
  }
  if (product.boundary) lines.push(`- ${markdownText(product.boundary, 320)}`);

  return lines.join("\n").replace(/\n{3,}/g, "\n\n").trim();
}

function studyHandoutFilename(label) {
  const safe = sanitizeText(label || "学习讲义", 80)
    .replace(/[\\/:*?"<>|#]+/g, "-")
    .replace(/\s+/g, "-")
    .replace(/^-+|-+$/g, "");
  return `${safe || "学习讲义"}.md`;
}

function markdownText(value, maxLength = 260) {
  return sanitizeText(value, maxLength).replace(/\|/g, "\\|");
}

function buildTopicEvidenceReportProduct(topic = {}) {
  const sourcePackage = topic.sourcePackage || {};
  const sourceControls = sourcePackage.sourceControls || {};
  const claimAudits = (Array.isArray(topic.claimSupport) ? topic.claimSupport : [])
    .filter((item) => item?.evidence?.length > 0)
    .slice(0, 5)
    .map((item, index) => {
      const evidence = item.evidence.slice(0, 3).map((entry) => evidenceReportEvidence(entry, item));
      return {
        id: sanitizeText(item.id || `claim-audit:${index}`, 120),
        claim: sanitizeText(item.claim, 260),
        verdict: "source_supported",
        verdictLabel: "有原文支撑",
        supportLabel: sanitizeText(item.supportLabel || "已采纳原文支撑", 80),
        evidenceCount: Number(item.evidenceCount || evidence.length || 0),
        dossierId: sanitizeText(item.dossierId, 80),
        dossierTitle: sanitizeText(item.dossierTitle, 120),
        evidence,
        gaps: buildEvidenceReportClaimGaps(topic),
        nextPrompt: sanitizeText(`请只基于已采纳来源解释这条结论，并指出它还不能证明什么：${item.claim}`, 360),
        boundary: "这条审计记录只说明已有原文支撑；它不能替代原文完整阅读，也不能替代你的业务验证。",
      };
    })
    .filter((item) => item.claim && item.evidence.length > 0);
  if (claimAudits.length === 0) return null;
  return {
    type: "evidence_report",
    title: `${sanitizeText(topic.label || "研究主题", 120)}可审计学习报告`,
    topicId: sanitizeText(topic.id, 80),
    topicLabel: sanitizeText(topic.label, 120),
    status: "source_backed",
    body: `把 ${claimAudits.length} 条来源支撑结论拆成可检查证据、待验证缺口和下一步追问。`,
    claimAudits,
    sourceLedger: buildEvidenceReportSourceLedger(claimAudits),
    sourceControls: {
      excludedSourceKeys: Array.isArray(sourceControls.excludedSourceKeys) ? sourceControls.excludedSourceKeys.slice(0, 30) : [],
      allowedAuthors: Array.isArray(sourceControls.allowedAuthors) ? sourceControls.allowedAuthors.slice(0, 20) : [],
      allowedSourceKeys: Array.isArray(sourceControls.allowedSourceKeys) ? sourceControls.allowedSourceKeys.slice(0, 50) : [],
      selectedSources: Array.isArray(sourceControls.selectedSources) ? sourceControls.selectedSources.slice(0, 12) : [],
    },
    nextPrompt: sourcePackage.nextPrompt || topic.nextActions?.[0] || "",
    boundary: "可审计学习报告只整理已采纳作者原文和待验证缺口；系统报告不能替代原文，用户业务材料和实验复盘也不会变成作者证据。",
  };
}

function evidenceReportEvidence(entry = {}, claim = {}) {
  return {
    label: sanitizeText(entry.label, 180),
    quote: sanitizeText(entry.quote, 260),
    author: sanitizeText(entry.author, 80),
    title: sanitizeText(entry.title, 180),
    date: sanitizeText(entry.date, 32),
    dossierId: sanitizeText(entry.dossierId || claim.dossierId, 80),
    sourceKey: sanitizeText(entry.sourceKey || sourceKeyForSource(entry), 260),
    sourcePath: sanitizeText(entry.sourcePath, 260),
    sourceUrl: sanitizeText(entry.sourceUrl, 260),
  };
}

function buildEvidenceReportClaimGaps(topic = {}) {
  const gaps = [
    "打开原文上下文，确认这条摘录是否真的支持当前结论。",
  ];
  if (Number(topic.materialRecords || 0) > 0) {
    gaps.push("已有业务材料，但仍要判断这些材料是否足以验证作者方法适用于你的产品。");
  } else {
    gaps.push("还缺你的产品、关键词、页面、广告或竞品材料，暂时不能判断是否适用于你的业务。");
  }
  if (Number(topic.experimentResults || 0) > 0) {
    gaps.push("已有实验复盘，但需要继续看样本量、周期和指标是否稳定。");
  } else {
    gaps.push("还缺小实验或复盘结果，暂时不能证明真实效果。");
  }
  return gaps.slice(0, 4);
}

function buildEvidenceReportSourceLedger(claimAudits = []) {
  const sources = new Map();
  for (const claim of Array.isArray(claimAudits) ? claimAudits : []) {
    for (const evidence of Array.isArray(claim.evidence) ? claim.evidence : []) {
      const key = sanitizeText(evidence.sourceKey || evidence.sourcePath || evidence.sourceUrl || evidence.label, 360);
      if (!key) continue;
      const row = sources.get(key) || {
        sourceKey: key,
        sourcePath: sanitizeText(evidence.sourcePath, 260),
        sourceUrl: sanitizeText(evidence.sourceUrl, 260),
        author: sanitizeText(evidence.author, 80),
        title: sanitizeText(evidence.title || evidence.label, 180),
        date: sanitizeText(evidence.date, 32),
        dossierId: sanitizeText(evidence.dossierId || claim.dossierId, 80),
        quote: sanitizeText(evidence.quote, 260),
        claimIds: [],
        claims: [],
        claimCount: 0,
      };
      if (claim.id && !row.claimIds.includes(claim.id)) row.claimIds.push(claim.id);
      if (claim.claim && !row.claims.includes(claim.claim)) row.claims.push(claim.claim);
      row.claimCount = row.claimIds.length || row.claims.length;
      sources.set(key, row);
    }
  }
  return [...sources.values()]
    .map((item) => ({
      ...item,
      claimIds: item.claimIds.slice(0, 8),
      claims: item.claims.slice(0, 4),
      claimCount: Number(item.claimCount || 0),
    }))
    .slice(0, 8);
}

function buildTopicStudyExecutionChecklist(topic = {}) {
  const path = Array.isArray(topic.learningPath) ? topic.learningPath : [];
  const kindByStep = {
    read_sources: "evidence",
    add_materials: "materials",
    run_experiment: "experiment",
    review_understanding: "review",
  };
  const rows = path.map((step) => ({
    id: sanitizeText(step.id, 80),
    kind: kindByStep[step.id] || "review",
    label: sanitizeText(step.label, 100),
    status: sanitizeText(step.status, 32),
    done: step.status === "done",
    reason: sanitizeText(step.reason, 220),
  }));
  if (rows.length > 0) return rows.slice(0, 6);
  return [
    {
      id: "read_sources",
      kind: "evidence",
      label: "先读来源证据",
      status: (topic.evidenceCount || 0) > 0 ? "done" : "current",
      done: (topic.evidenceCount || 0) > 0,
      reason: "先确认这个主题是否有已采纳作者原文证据。",
    },
  ];
}

function buildTopicStudyReviewQuestions(topic = {}, sourceBackedClaims = []) {
  const claimQuestions = sourceBackedClaims.slice(0, 3).map((item, index) => ({
    id: `claim-review:${index}`,
    question: `为什么说“${item.claim}”？`,
    expectedAnswer: item.claim,
    sourceHint: item.evidence[0]?.label || item.supportLabel,
    prompt: `请只基于已采纳来源解释这个结论：${item.claim}`,
  }));
  const actionQuestions = uniqueOrdered(topic.nextActions || [], 3).map((item, index) => ({
    id: `action-review:${index}`,
    question: `下一步为什么要做“${item}”？`,
    expectedAnswer: item,
    sourceHint: "需要结合已采纳来源和你的产品材料验证。",
    prompt: `请把“${item}”拆成可执行检查清单，并区分作者来源和我的业务验证。`,
  }));
  return [...claimQuestions, ...actionQuestions].slice(0, 6);
}

function detectResearchTopic(dossier) {
  const text = [
    dossier.title,
    dossier.question,
    dossier.takeaway,
    ...dossier.conclusions,
    ...dossier.nextActions,
    ...dossier.missingInputs,
    ...dossier.followUps,
    ...dossier.acceptedEvidence.map((item) => `${item.title} ${item.quote || item.text || ""}`),
    ...dossier.businessVerificationRecords.flatMap((record) => [
      record.rawText,
      ...(Array.isArray(record.sections) ? record.sections.flatMap((section) => section.items || []) : []),
    ]),
  ].join("\n").toLowerCase();
  let best = null;
  let bestScore = 0;
  for (const topic of RESEARCH_TOPIC_DEFS) {
    const score = topic.keywords.reduce((count, keyword) => count + (text.includes(String(keyword).toLowerCase()) ? 1 : 0), 0);
    if (score > bestScore) {
      best = topic;
      bestScore = score;
    }
  }
  return best || { id: "general", label: "综合学习主题", keywords: [] };
}

function formatEvidenceNote(evidence) {
  const source = [evidence.author, evidence.title].filter(Boolean).join("《");
  const sourceText = evidence.author && evidence.title ? `${source}》` : source || evidence.title || "来源";
  const quote = sanitizeText(evidence.quote || evidence.text || "", 180);
  return quote ? `${sourceText}：${quote}` : sourceText;
}

function mergeTopicScope(current, authors = []) {
  const safeAuthors = normalizeAuthorNames(authors);
  if (safeAuthors.length === 0) return current || "全部作者";
  if (!current || current === "全部作者") return `只看 ${safeAuthors.join("、")}`;
  const existing = current.replace(/^只看\s*/, "").split("、").filter(Boolean);
  return `只看 ${uniqueOrdered([...existing, ...safeAuthors], 5).join("、")}`;
}

function faqAnswerHint(question, topicGroups = []) {
  const topic = topicGroups.find((group) => textMatchesTopic(question, group));
  if (topic?.keyTakeaways?.length) return topic.keyTakeaways[0];
  if (topic?.nextActions?.length) return topic.nextActions[0];
  return "先用本地资料回答，再把具体产品、关键词或页面材料补进档案验证。";
}

function textMatchesTopic(text, group) {
  const def = RESEARCH_TOPIC_DEFS.find((topic) => topic.id === group?.id);
  if (!def) return false;
  const value = String(text || "").toLowerCase();
  return def.keywords.some((keyword) => value.includes(String(keyword).toLowerCase()));
}

function buildOverviewGapLines(totals = {}, topicGroups = []) {
  const lines = [];
  if ((totals.dossiersWithoutAcceptedEvidence || 0) > 0) lines.push(`${totals.dossiersWithoutAcceptedEvidence} 个档案还缺已采纳原文证据。`);
  if ((totals.businessVerificationOpen || 0) > 0) lines.push(`${totals.businessVerificationOpen} 个档案还没补产品材料。`);
  if ((totals.experimentResults || 0) === 0) lines.push("还没有实验复盘，暂时不能把建议当成已验证策略。");
  const thinTopic = topicGroups.find((group) => group.evidenceCount === 0 || group.materialRecords === 0);
  if (thinTopic) lines.push(`${thinTopic.label} 主题还需要补证据或业务材料。`);
  return lines.length ? uniqueOrdered(lines, 5) : ["证据、材料和实验都已有基础记录，可以进入复盘和自测。"];
}

function missionStagePriority(stage) {
  return {
    evidence: 0,
    materials: 1,
    verification: 2,
    experiment: 3,
    review: 4,
    self_test: 5,
    ready: 6,
  }[stage] ?? 9;
}

export function updateDossierReviewState(value = {}, patch = {}) {
  const dossier = normalizeStoredDossier(value);
  const queue = buildReviewQueue(dossier);
  const allowedIds = new Set(queue.items.filter((item) => item.canManualComplete !== false).map((item) => item.id));
  const checked = { ...dossier.reviewState.checked };
  const updates = patch?.checked && typeof patch.checked === "object" ? patch.checked : {};

  Object.entries(updates).forEach(([rawId, rawValue]) => {
    const id = sanitizeText(rawId, 140);
    if (!allowedIds.has(id)) return;
    if (rawValue === true) checked[id] = true;
    else delete checked[id];
  });

  return normalizeStoredDossier({
    ...dossier,
    reviewState: {
      checked,
      updatedAt: sanitizeText(patch?.updatedAt || new Date().toISOString(), 40),
    },
  });
}

export function updateDossierSelfTestState(value = {}, patch = {}) {
  const dossier = normalizeStoredDossier(value);
  const selfTest = buildSelfTest(dossier);
  const allowedIds = new Set(selfTest.items.map((item) => item.id));
  const mastered = { ...dossier.selfTestState.mastered };
  const updates = patch?.mastered && typeof patch.mastered === "object" ? patch.mastered : {};

  Object.entries(updates).forEach(([rawId, rawValue]) => {
    const id = sanitizeText(rawId, 140);
    if (!allowedIds.has(id)) return;
    if (rawValue === true) mastered[id] = true;
    else delete mastered[id];
  });

  return normalizeStoredDossier({
    ...dossier,
    selfTestState: {
      mastered,
      updatedAt: sanitizeText(patch?.updatedAt || new Date().toISOString(), 40),
    },
  });
}

export function updateDossierBusinessVerificationState(value = {}, patch = {}) {
  const dossier = normalizeStoredDossier(value);
  const record = buildBusinessVerificationRecord(dossier, {
    text: patch?.text,
    createdAt: patch?.createdAt || new Date().toISOString(),
  });
  if (!record) return dossier;

  const records = [
    record,
    ...dossier.businessVerificationRecords.filter((item) => item.rawText !== record.rawText && item.id !== record.id),
  ].slice(0, 8);

  return normalizeStoredDossier({
    ...dossier,
    businessVerificationRecords: records,
  });
}

export function updateDossierExperimentResultState(value = {}, patch = {}) {
  const dossier = normalizeStoredDossier(value);
  const record = buildExperimentResultRecord({
    text: patch?.text,
    createdAt: patch?.createdAt || new Date().toISOString(),
  });
  if (!record) return dossier;

  const records = [
    record,
    ...dossier.experimentResultRecords.filter((item) => item.rawText !== record.rawText && item.id !== record.id),
  ].slice(0, 8);

  return normalizeStoredDossier({
    ...dossier,
    experimentResultRecords: records,
  });
}

export function buildProductIntake(input = {}, dossierValue = {}) {
  const dossier = normalizeStoredDossier(dossierValue);
  const text = String(input.text || "").slice(0, 3000);
  const sections = PRODUCT_INTAKE_SECTIONS.map((section) => ({
    id: section.id,
    label: section.label,
    items: [],
    missing: section.required.slice(0, 4),
  }));

  splitProductFacts(text).forEach((fact) => {
    const sectionId = bestProductSectionId(fact);
    const section = sections.find((item) => item.id === sectionId) || sections[sections.length - 1];
    section.items.push(fact);
  });

  sections.forEach((section) => {
    section.items = section.items.slice(0, 8);
    section.missing = section.missing.filter((item) => !sectionHasRequired(section, item)).slice(0, 4);
  });

  const usefulSections = sections.filter((section) => section.id !== "other" && section.items.length > 0);
  const dossierMissing = dossier.missingInputs.filter((item) => !text.includes(item)).slice(0, 4);
  const missing = [...new Set([...sections.flatMap((section) => section.missing), ...dossierMissing])].slice(0, 10);
  const diagnosticPrompt = buildDiagnosticPrompt(dossier, usefulSections, missing);

  return {
    summary: usefulSections.length > 0
      ? `已识别 ${usefulSections.length} 类产品信息，适合进入下一轮诊断。`
      : "还没有识别到足够具体的产品信息，建议补充主图、数据、Listing、广告或关键词。",
    sections,
    missing,
    diagnosticPrompt,
    caution: "这是本地关键词归类，不会判断图片好坏，也不会写入原始知识库。",
  };
}

export function buildDossierSessionSeed(value = {}) {
  const dossier = normalizeStoredDossier(value);
  const sources = collectSessionSources(dossier);
  const sourceIndexByKey = new Map();
  const excludedSourceKeySet = new Set(dossier.excludedSources.map((item) => item.key).filter(Boolean));
  sources.forEach((source, index) => {
    sourceIdentityKeys(source).forEach((key) => sourceIndexByKey.set(key, index));
  });

  const claims = [];
  const evidenceFeedback = {};
  const addEvidence = (item, value) => {
    const sourceKey = item.sourceKey || sourceKeyForSource(item);
    if (excludedSourceKeySet.has(sourceKey)) return;
    const id = `source-evidence:${claims.length}`;
    const sourceIndex = sourceIndexByKey.has(sourceKey) ? sourceIndexByKey.get(sourceKey) : undefined;
    claims.push({
      id,
      type: "source_evidence",
      label: "资料证据",
      canProve: true,
      evidenceKind: "source_quote",
      text: sanitizeText(item.text || item.quote, 260),
      quote: sanitizeText(item.quote || item.text, 700),
      sourceIndex,
      author: sanitizeText(item.author, 80),
      title: sanitizeText(item.title, 180),
      date: sanitizeText(item.date, 32),
      basis: "来自已保存学习档案中的原文证据快照",
    });
    evidenceFeedback[id] = value;
  };
  dossier.acceptedEvidence.forEach((item) => addEvidence(item, "useful"));
  dossier.rejectedEvidence.forEach((item) => addEvidence(item, "irrelevant"));

  if (claims.length === 0) {
    claims.push({
      id: "needs-source:0",
      type: "needs_source",
      label: "暂无直接证据",
      canProve: false,
      evidenceKind: "missing_source",
      text: "这个学习档案没有保存可直接复用的原文证据。",
      validation: "继续追问时建议补充具体产品、关键词、页面或广告数据。",
    });
  }

  const createdAt = dossier.createdAt || new Date().toISOString();
  const assistantMessage = {
    role: "assistant",
    content: restoredContent(dossier),
    sources,
    evidenceChain: {
      summary: `从学习档案恢复：采纳证据 ${dossier.acceptedEvidence.length} 条，排除证据 ${dossier.rejectedEvidence.length} 条，排除来源 ${dossier.excludedSources.length} 篇。`,
      claims,
    },
    evidenceFeedback,
    sourceScope: dossier.allowedAuthors.length > 0
      ? {
          active: true,
          allowedAuthors: dossier.allowedAuthors,
          totalRetrieved: dossier.sources.length,
          totalAfterScope: dossier.sources.filter((source) => dossier.allowedAuthors.includes(source.author)).length,
          summary: `这个学习档案恢复为只使用 ${dossier.allowedAuthors.join("、")} 的资料。`,
          caution: "研究范围只影响继续追问，不会删除或改写原始知识库。",
        }
      : {
          active: false,
          allowedAuthors: [],
          totalRetrieved: dossier.sources.length,
          totalAfterScope: dossier.sources.length,
          summary: "这个学习档案恢复为使用全部作者资料。",
          caution: "你可以继续在左侧按作者锁定研究范围。",
    },
    productInputSummary: dossier.productInputSummary,
    diagnosisPanel: dossier.diagnosisPanel,
    synthesisAnswer: dossier.synthesisAnswer,
    workflowIntent: dossier.workflowIntent || {
      type: "method_learning",
      label: "方法学习",
      goal: "从已保存学习档案恢复上下文，继续核对作者来源和下一步动作。",
      primaryAction: "先确认档案里的来源证据，再继续追问或补产品数据。",
      boundary: "学习档案是本地沉淀，不会改写作者原文；继续追问仍要回到作者来源核对。",
      confidence: "medium",
    },
    learningCard: {
      intent: {
        type: "archive",
        label: dossier.title || "已保存学习档案",
        description: "从本机学习档案恢复的上下文。",
      },
      takeaway: dossier.takeaway,
      conclusions: dossier.conclusions,
      nextActions: dossier.nextActions,
      missingInputs: dossier.missingInputs,
      followUps: dossier.followUps,
      evidence: dossier.acceptedEvidence.map((item, index) => ({
        sourceIndex: sourceIndexByKey.get(item.sourceKey || sourceKeyForSource(item)) ?? index,
        title: item.title,
        author: item.author,
        date: item.date,
      })),
    },
    createdAt,
    restoredFromDossierId: dossier.id,
  };

  return {
    messages: [
      { role: "user", content: dossier.question || dossier.title || "继续这个学习档案", createdAt },
      assistantMessage,
    ],
    sourceControls: {
      excludedSourceKeys: dossier.excludedSources.map((item) => item.key).filter(Boolean).slice(0, 100),
      allowedAuthors: dossier.allowedAuthors.slice(0, 20),
    },
  };
}

const PRODUCT_INTAKE_SECTIONS = [
  {
    id: "visual",
    label: "主图/视觉",
    keywords: ["主图", "副图", "图片", "视觉", "场景图", "对比图", "video", "image", "photo", "infographic"],
    required: ["主图/副图截图", "竞品主图对照", "核心卖点画面"],
  },
  {
    id: "metrics",
    label: "点击率/转化率数据",
    keywords: ["点击率", "转化率", "ctr", "cvr", "sessions", "session", "流量", "曝光", "impression", "订单", "销量", "转化"],
    required: ["CTR", "CVR", "时间窗口", "流量来源"],
  },
  {
    id: "listing",
    label: "Listing/页面",
    keywords: ["listing", "标题", "五点", "bullet", "描述", "文案", "a+", "页面", "review", "评价", "rating", "价格"],
    required: ["标题", "五点", "价格", "评价数量"],
  },
  {
    id: "ads",
    label: "广告/流量",
    keywords: ["广告", "acos", "ppc", "sp", "sbv", "预算", "竞价", "bid", "campaign", "关键词广告", "投放"],
    required: ["广告类型", "ACOS", "预算", "主要花费词"],
  },
  {
    id: "keywords",
    label: "关键词/竞品",
    keywords: ["关键词", "搜索词", "竞品", "竞对", "asin", "类目", "排名", "keyword", "competitor", "search term"],
    required: ["核心关键词", "竞品 ASIN", "类目位置", "搜索结果截图"],
  },
  {
    id: "other",
    label: "缺口",
    keywords: [],
    required: [],
  },
];

const BUSINESS_PROMPT_EVIDENCE_BOUNDARY = "边界：只用作者原文证据支撑作者观点；用户业务材料只用于验证适配性，不能把用户材料当成作者原文证据；实验复盘不能改写作者资料。";

function splitProductFacts(text) {
  return String(text || "")
    .split(/[\n；;，,]+/g)
    .map((item) => sanitizeText(item, 220))
    .filter(Boolean)
    .slice(0, 36);
}

function bestProductSectionId(fact) {
  const value = String(fact || "").toLowerCase();
  if (hasMetricData(value)) return "metrics";
  if (hasAdData(value)) return "ads";
  if (hasVisualMaterial(value)) return "visual";
  if (hasListingMaterial(value)) return "listing";
  if (hasKeywordOrCompetitorData(value)) return "keywords";
  return "other";
}

function hasMetricData(value) {
  return (
    (/\bctr\b|点击率|cvr|转化率|session|sessions|流量|曝光|impression/.test(value) && /\d+(\.\d+)?\s*%?/.test(value)) ||
    /\d+(\.\d+)?\s*%\s*(ctr|cvr|点击率|转化率)/.test(value)
  );
}

function hasAdData(value) {
  return (
    (/\bacos\b|广告|ppc|sp\b|sbv|预算|竞价|bid|campaign/.test(value) && /\d+(\.\d+)?\s*%?/.test(value)) ||
    /acos\s*\d+/.test(value)
  );
}

function hasKeywordOrCompetitorData(value) {
  return /\basin\b|b0[0-9a-z]{8}|竞品链接|竞品 asin|核心关键词|搜索词[:：]|keyword[:：]|关键词[:：]/i.test(value);
}

function hasVisualMaterial(value) {
  return (
    /https?:\/\/\S+\.(png|jpe?g|webp|gif)/i.test(value) ||
    /(主图|副图|图片|视觉|场景图|对比图|截图).*(白底|场景|对比|卖点|文字|模特|包装|尺寸|链接|截图|上传|是|有)/.test(value)
  );
}

function hasListingMaterial(value) {
  return /(listing|标题|五点|bullet|a\+|页面|描述|文案|价格|评价|rating|review).*(写|有|是|包含|数量|星|短|长|\\$|￥|¥|\\d)/i.test(value);
}

function sectionHasRequired(section, required) {
  const value = `${section.items.join(" ")} ${section.label}`.toLowerCase();
  return required
    .split(/[\/、\s]+/g)
    .filter(Boolean)
    .some((part) => value.includes(part.toLowerCase()));
}

function buildDiagnosticPrompt(dossier, usefulSections, missing) {
  const lines = [
    "我补充了以下产品信息，请结合当前学习档案、已采纳证据和已排除来源，判断我现在应该先改哪一块：",
  ];
  usefulSections.forEach((section) => {
    lines.push("", `${section.label}：`);
    section.items.slice(0, 5).forEach((item, index) => lines.push(`${index + 1}. ${item}`));
  });
  if (missing.length > 0) {
    lines.push("", `仍缺信息：${missing.slice(0, 6).join("、")}`);
  }
  if (dossier.takeaway) {
    lines.push("", `当前学习档案结论：${dossier.takeaway}`);
  }
  lines.push("", BUSINESS_PROMPT_EVIDENCE_BOUNDARY);
  lines.push("", "请输出：1. 最先检查项；2. 为什么先看它；3. 下一步需要我补什么材料。");
  return sanitizeText(lines.join("\n"), 1800);
}

function buildQuestionPack(dossier) {
  const questions = [];
  const addQuestion = (intent, label, question) => {
    const safeQuestion = sanitizeText(question, 360);
    if (!safeQuestion || questions.some((item) => item.question === safeQuestion)) return;
    questions.push({
      id: `${intent}:${questions.filter((item) => item.intent === intent).length}`,
      intent,
      label: sanitizeText(label, 80),
      question: safeQuestion,
    });
  };

  dossier.followUps.slice(0, 5).forEach((item, index) => {
    addQuestion("follow_up", `追问 ${index + 1}`, item);
  });

  if (dossier.missingInputs.length > 0) {
    addQuestion(
      "diagnose",
      "补充信息后重诊断",
      `我补充这些信息：${dossier.missingInputs.slice(0, 4).join("、")}。请基于这个学习档案重新判断下一步。`,
    );
  }

  if (dossier.nextActions.length > 0) {
    addQuestion(
      "sequence",
      "执行顺序复核",
      `请把这个学习档案转成我的执行检查表，并按优先级解释为什么：${dossier.nextActions.slice(0, 5).join("、")}。`,
    );
  }

  addQuestion(
    "next_best_step",
    "只问下一步",
    "结合这个学习档案、已采纳证据和已排除来源，如果我现在只改一个点，应该先改哪一块？",
  );

  return questions.slice(0, 8);
}

function collectSessionSources(dossier) {
  const byKey = new Map();
  const excludedSourceKeySet = new Set(dossier.excludedSources.map((item) => item.key).filter(Boolean));
  const addSource = (source) => {
    const normalized = normalizeSource(source);
    if (sourceIdentityKeys(normalized).some((key) => excludedSourceKeySet.has(key))) return;
    const key = sourceKeyForSource(normalized);
    if (!key || byKey.has(key)) return;
    byKey.set(key, normalized);
  };
  dossier.sources.forEach(addSource);
  [...dossier.acceptedEvidence, ...dossier.rejectedEvidence].forEach(addSource);
  return [...byKey.values()].slice(0, 12);
}

function restoredContent(dossier) {
  const lines = [
    `问题：${dossier.question || dossier.title}`,
    "",
    "已从学习档案恢复上下文。后续追问会优先参考已采纳证据，并避开已排除来源。",
  ];
  if (dossier.takeaway) {
    lines.push("", "档案结论：", `1. ${dossier.takeaway}`);
  }
  if (dossier.nextActions.length > 0) {
    lines.push("", "行动顺序：");
    dossier.nextActions.slice(0, 5).forEach((item, index) => lines.push(`${index + 1}. ${item}`));
  }
  if (dossier.acceptedEvidence.length > 0) {
    lines.push("", "已采纳证据：");
    dossier.acceptedEvidence.slice(0, 3).forEach((item, index) => {
      lines.push(`${index + 1}. ${item.quote || item.text}（${item.author}《${item.title}》）`);
    });
  } else {
    lines.push("", "已采纳证据：暂无。");
  }
  if (dossier.excludedSources.length > 0) {
    lines.push("", `已排除来源：${dossier.excludedSources.length} 篇。`);
  }
  if (dossier.allowedAuthors.length > 0) {
    lines.push("", `研究范围：只使用 ${dossier.allowedAuthors.join("、")} 的资料继续追问。`);
  }
  const diagnosis = buildDossierDiagnosisSummary(dossier);
  if (diagnosis) {
    lines.push(
      "",
      "已保存产品诊断：",
      `1. ${diagnosis.priority || "已保存诊断排查面板"}（保存时排查记录 ${diagnosis.checkedChecks}/${diagnosis.totalChecks} 项，不是证据）`,
    );
  }
  if (dossier.experimentResultRecords.length > 0) {
    lines.push("", "已保存实验复盘：");
    dossier.experimentResultRecords.slice(0, 3).forEach((item, index) => {
      lines.push(`${index + 1}. ${item.summary}；下一步：${item.nextAction}（用户回填，不是作者原文证据）`);
    });
  }
  return lines.join("\n");
}

function normalizeEvidenceClaim(claim, source) {
  const normalizedSource = normalizeSource({
    ...source,
    author: claim.author || source.author,
    title: claim.title || source.title,
    date: claim.date || source.date,
  });
  return {
    id: sanitizeText(claim.id, 80),
    quote: sanitizeText(claim.quote || claim.text, 700),
    text: sanitizeText(claim.text || claim.quote, 260),
    author: normalizedSource.author,
    title: normalizedSource.title,
    date: normalizedSource.date,
    sourceUrl: normalizedSource.sourceUrl,
    sourcePath: normalizedSource.sourcePath,
    sourceKey: sourceKeyForSource(normalizedSource),
  };
}

function normalizeEvidenceList(items, limit) {
  if (!Array.isArray(items)) return [];
  return items
    .map((item) => ({
      id: sanitizeText(item?.id, 80),
      quote: sanitizeText(item?.quote, 700),
      text: sanitizeText(item?.text, 260),
      author: sanitizeText(item?.author, 80),
      title: sanitizeText(item?.title, 180),
      date: sanitizeText(item?.date, 32),
      sourceUrl: sanitizeText(item?.sourceUrl, 260),
      sourcePath: sanitizeText(item?.sourcePath, 260),
      sourceKey: sanitizeText(item?.sourceKey, 360),
    }))
    .filter((item) => item.quote || item.text)
    .slice(0, limit);
}

function normalizeExcludedSources(items, limit) {
  if (!Array.isArray(items)) return [];
  return items
    .map((item) => ({
      key: sanitizeText(item?.key, 360),
      label: sanitizeText(item?.label, 220),
    }))
    .filter((item) => item.key)
    .slice(0, limit);
}

function normalizeSource(source = {}) {
  return {
    author: sanitizeText(source.author, 80),
    date: sanitizeText(source.date, 32),
    title: sanitizeText(source.title, 180),
    sourceUrl: sanitizeText(source.sourceUrl, 260),
    sourcePath: sanitizeText(source.sourcePath, 260),
    sourceType: sanitizeText(source.sourceType, 60),
    excerpt: sanitizeText(source.excerpt, 500),
  };
}

function isAuthorEvidenceSource(source = {}) {
  const author = sanitizeText(source.author, 80);
  if (author === "我的资料") return false;
  if (String(source.sourceUrl || "").startsWith("user-source://")) return false;
  if (String(source.sourcePath || "").startsWith("user-sources/")) return false;
  if (String(source.sourceType || "") === "user_material") return false;
  return Boolean(author && source.title);
}

function sourceContainsQuote(source, quote) {
  const sourceText = normalizeEvidenceSourceText(source.excerpt);
  const quoteText = normalizeEvidenceSourceText(quote);
  if (!sourceText || !quoteText) return false;
  return sourceText.includes(quoteText);
}

function normalizeEvidenceSourceText(value) {
  return String(value || "")
    .replace(/[【】\[\]（）()《》「」“”"'`]/g, "")
    .replace(/\s+/g, "")
    .trim();
}

function normalizeProductInputSummary(summary) {
  if (!summary || typeof summary !== "object") return undefined;
  const facts = Array.isArray(summary.facts)
    ? summary.facts
        .map((section) => ({
          id: sanitizeText(section?.id, 40),
          label: sanitizeText(section?.label || "产品信息", 80),
          items: sanitizeList(section?.items, 6, 220),
          missing: sanitizeList(section?.missing, 4, 120),
        }))
        .filter((section) => section.items.length > 0)
        .slice(0, 8)
    : [];
  const missing = sanitizeList(summary.missing, 8, 120);
  const summaryText = sanitizeText(summary.summary, 180);
  if (facts.length === 0 && missing.length === 0 && !summaryText) return undefined;
  return {
    source: "user_input",
    summary: summaryText,
    facts,
    missing,
    caution: sanitizeText(summary.caution || "这些是用户提供的诊断输入，不是本地资料证据。", 220),
  };
}

function normalizeWorkflowIntent(intent) {
  if (!intent || typeof intent !== "object") return undefined;
  const type = sanitizeText(intent.type || "method_learning", 60);
  const label = sanitizeText(intent.label || "方法学习", 80);
  if (!type && !label) return undefined;
  return {
    type,
    label,
    goal: sanitizeText(intent.goal, 220),
    primaryAction: sanitizeText(intent.primaryAction, 220),
    nextPrompt: sanitizeText(intent.nextPrompt, 420),
    boundary: sanitizeText(
      intent.boundary || "本轮意图只用于安排学习动作，不会改变作者原文和用户业务记录的边界。",
      260,
    ),
    confidence: sanitizeText(intent.confidence || "medium", 20),
  };
}

function normalizeValidationPack(pack) {
  if (!pack || typeof pack !== "object") return undefined;
  const status = pack.status === "source_backed" || pack.status === "needs_source"
    ? pack.status
    : "needs_source";
  const hypotheses = Array.isArray(pack.hypotheses)
    ? pack.hypotheses
        .map((item, index) => ({
          id: sanitizeText(item?.id || `hypothesis:${index}`, 80),
          label: sanitizeText(item?.label || item?.quote || item?.text, 140),
          sourceIndex: Number.isInteger(item?.sourceIndex) && item.sourceIndex >= 0 ? item.sourceIndex : undefined,
          author: sanitizeText(item?.author, 80),
          sourceTitle: sanitizeText(item?.sourceTitle || item?.title, 160),
          quote: sanitizeText(item?.quote || item?.text, 240),
          verifyWith: sanitizeText(item?.verifyWith, 220),
        }))
        .filter((item) => item.label || item.quote)
        .slice(0, 4)
    : [];
  const dataRequests = Array.isArray(pack.dataRequests)
    ? pack.dataRequests
        .map((item, index) => ({
          id: sanitizeText(item?.id || `data:${index}`, 80),
          label: sanitizeText(item?.label, 100),
          why: sanitizeText(item?.why, 180),
          placeholder: sanitizeText(item?.placeholder, 140),
        }))
        .filter((item) => item.label)
        .slice(0, 6)
    : [];
  const experiments = Array.isArray(pack.experiments)
    ? pack.experiments
        .map((item, index) => ({
          id: sanitizeText(item?.id || `experiment:${index}`, 80),
          title: sanitizeText(item?.title, 130),
          steps: sanitizeList(item?.steps, 4, 140),
          successSignal: sanitizeText(item?.successSignal, 220),
        }))
        .filter((item) => item.title)
        .slice(0, 3)
    : [];
  const decisionRules = Array.isArray(pack.decisionRules)
    ? pack.decisionRules
        .map((item) => ({
          if: sanitizeText(item?.if, 160),
          then: sanitizeText(item?.then, 220),
        }))
        .filter((item) => item.if || item.then)
        .slice(0, 5)
    : [];

  if (hypotheses.length === 0 && dataRequests.length === 0 && experiments.length === 0 && decisionRules.length === 0) {
    return undefined;
  }

  return {
    title: sanitizeText(pack.title || "本轮业务验证任务包", 120),
    status,
    summary: sanitizeText(pack.summary, 260),
    boundary: sanitizeText(
      pack.boundary || "任务包里的用户数据只用于复核，不会变成作者原文证据。",
      320,
    ),
    hypotheses,
    dataRequests,
    experiments,
    decisionRules,
    businessDecision: normalizeBusinessDecision(pack.businessDecision),
    followUpPrompt: sanitizeText(pack.followUpPrompt, 520),
  };
}

function normalizeSynthesisAnswer(value, sources = []) {
  if (!value || typeof value !== "object") return undefined;
  const validSources = Array.isArray(sources) ? sources.map(normalizeSource) : [];
  const validSourceIndex = (index) => Number.isInteger(index) && index >= 0 && index < validSources.length;
  const uniqueNumberOrdered = (items, limit = 6) => {
    const seen = new Set();
    const rows = [];
    for (const item of Array.isArray(items) ? items : []) {
      const index = Number(item);
      if (!validSourceIndex(index) || seen.has(index)) continue;
      seen.add(index);
      rows.push(index);
      if (rows.length >= limit) break;
    }
    return rows;
  };
  const normalizeSourceIndexes = (indexes, limit = 6) => {
    if (!Array.isArray(indexes)) return [];
    return uniqueNumberOrdered(indexes, limit);
  };
  const normalizeSupport = (items) => {
    if (!Array.isArray(items)) return [];
    return items
      .map((item) => {
        const sourceIndex = Number(item?.sourceIndex);
        if (!validSourceIndex(sourceIndex)) return null;
        const source = validSources[sourceIndex] || {};
        return {
          claimId: sanitizeText(item?.claimId, 80),
          sourceIndex,
          identity: sanitizeText(item?.identity || "作者原文", 60),
          evidenceKind: sanitizeText(item?.evidenceKind || "source_evidence", 60),
          author: sanitizeText(item?.author || source.author, 80),
          title: sanitizeText(item?.title || source.title, 180),
          date: sanitizeText(item?.date || source.date, 32),
          quote: sanitizeText(item?.quote || item?.text || source.excerpt, 260),
        };
      })
      .filter((item) => item && (item.title || item.quote))
      .slice(0, 4);
  };
  const points = Array.isArray(value.points)
    ? value.points
        .map((point, index) => {
          const support = normalizeSupport(point?.support);
          const supportClaimIds = new Set(support.map((item) => item.claimId).filter(Boolean));
          const claimIds = sanitizeList(point?.claimIds, 8, 80).filter((id) => supportClaimIds.has(id));
          const sourceIndexes = uniqueNumberOrdered([
            ...support.map((item) => item.sourceIndex),
          ], 6);
          return {
            id: sanitizeText(point?.id || `synthesis-point:${index}`, 80),
            label: sanitizeText(point?.label, 140),
            text: sanitizeText(point?.text, 360),
            identity: sanitizeText(point?.identity || "系统综合", 60),
            evidenceKind: "system_synthesis",
            canUseAsEvidence: false,
            isInference: point?.isInference !== false,
            confidence: ["high", "medium", "low"].includes(point?.confidence) ? point.confidence : "medium",
            basis: sanitizeText(point?.basis || "系统综合，不是新的作者原文证据。", 260),
            claimIds,
            sourceIndexes,
            support,
          };
        })
        .filter((point) => point.label || point.text || point.support.length > 0)
        .slice(0, 6)
    : [];
  const authorPerspectives = Array.isArray(value.authorPerspectives)
    ? value.authorPerspectives
        .map((item, index) => ({
          id: sanitizeText(item?.id || `synthesis-author:${index}`, 80),
          author: sanitizeText(item?.author, 80),
          summary: sanitizeText(item?.summary || item?.stance, 260),
          sourceIndexes: normalizeSourceIndexes(item?.sourceIndexes, 6),
          claimIds: sanitizeList(item?.claimIds, 8, 80),
        }))
        .filter((item) => item.author || item.summary)
        .slice(0, 5)
    : [];
  const conflicts = Array.isArray(value.conflicts)
    ? value.conflicts
        .map((item, index) => ({
          id: sanitizeText(item?.id || `synthesis-conflict:${index}`, 80),
          concept: sanitizeText(item?.concept || item?.label, 120),
          message: sanitizeText(item?.message || item?.reason, 260),
          sourceIndexes: normalizeSourceIndexes(item?.sourceIndexes, 6),
        }))
        .filter((item) => item.concept || item.message)
        .slice(0, 4)
    : [];
  const gaps = Array.isArray(value.gaps)
    ? value.gaps
        .map((item, index) => ({
          id: sanitizeText(item?.id || `synthesis-gap:${index}`, 80),
          label: sanitizeText(item?.label, 120),
          reason: sanitizeText(item?.reason || item?.message, 260),
        }))
        .filter((item) => item.label || item.reason)
        .slice(0, 6)
    : [];
  const supportSourceIndexes = uniqueNumberOrdered(points.flatMap((point) => point.sourceIndexes), 20);
  const supportRows = points.flatMap((point) => point.support);
  const supportClaimIds = uniqueOrdered(supportRows.map((item) => item.claimId).filter(Boolean), 20);
  const supportAuthors = uniqueOrdered(
    supportRows.map((item) => item.author).filter(Boolean),
    8,
  );
  const hasEffectiveSupport = points.some((point) => point.support.length > 0);
  const status = hasEffectiveSupport
    ? (value.status === "needs_review" ? "needs_review" : "source_backed")
    : "needs_source";

  if (points.length === 0 && authorPerspectives.length === 0 && conflicts.length === 0 && gaps.length === 0 && !value.summary) {
    return undefined;
  }

  return {
    title: sanitizeText(value.title || "本轮综合答案", 120),
    status,
    summary: sanitizeText(value.summary, 300),
    sourceCoverage: {
      sourceCount: supportSourceIndexes.length,
      evidenceCount: supportClaimIds.length,
      authorCount: supportAuthors.length,
      authors: supportAuthors,
    },
    sourceClaimIds: supportClaimIds,
    points,
    authorPerspectives,
    conflicts,
    gaps,
    boundary: sanitizeText(
      value.boundary || "系统综合，不是新的作者原文证据；只有绑定的作者摘录可作为来源支撑。",
      320,
    ),
  };
}

function buildFallbackSynthesisAnswer(input = {}) {
  const hasContent = Boolean(
    input.takeaway ||
    input.answerPreview ||
    (Array.isArray(input.conclusions) && input.conclusions.length > 0) ||
    (Array.isArray(input.nextActions) && input.nextActions.length > 0),
  );
  if (!hasContent) return undefined;
  const acceptedEvidence = Array.isArray(input.acceptedEvidence) ? input.acceptedEvidence : [];
  const sources = Array.isArray(input.sources) ? input.sources : [];
  const support = acceptedEvidence.slice(0, 3).map((item, index) => ({
    claimId: sanitizeText(item.id || `accepted:${index}`, 80),
    sourceIndex: matchingSourceIndexForEvidence(item, sources),
    identity: "作者原文",
    evidenceKind: "source_evidence",
    author: sanitizeText(item.author, 80),
    title: sanitizeText(item.title, 180),
    date: sanitizeText(item.date, 32),
    quote: sanitizeText(item.quote || item.text, 260),
  })).filter((item) => Number.isInteger(item.sourceIndex) && item.sourceIndex >= 0 && (item.title || item.quote));
  const pointTexts = uniqueOrdered(
    [
      input.takeaway,
      ...(Array.isArray(input.conclusions) ? input.conclusions : []),
      ...(Array.isArray(input.nextActions) ? input.nextActions : []).map((item) => `下一步：${item}`),
    ],
    6,
  );
  const points = pointTexts.map((text, index) => ({
    id: `fallback-synthesis-point:${index}`,
    label: index === 0 ? "保存时结论" : "保存时要点",
    text,
    identity: "系统综合",
    evidenceKind: "system_synthesis",
    canUseAsEvidence: false,
    isInference: true,
    confidence: "low",
    basis: support.length > 0
      ? "由旧档案中的已采纳证据和回答快照恢复；建议继续回到来源复核。"
      : "由旧档案回答快照恢复；还没有绑定已采纳作者原文证据。",
    claimIds: support.map((item) => item.claimId).filter(Boolean),
    sourceIndexes: support.map((item) => item.sourceIndex),
    support,
  }));
  const gaps = [];
  if (support.length === 0) {
    gaps.push({
      id: "fallback-synthesis-gap:evidence",
      label: "待确认作者来源",
      reason: "这个旧档案没有可绑定的已采纳作者原文证据，继续学习前应先确认来源。",
    });
  }
  if (Array.isArray(input.missingInputs) && input.missingInputs.length > 0) {
    gaps.push({
      id: "fallback-synthesis-gap:input",
      label: "待补业务材料",
      reason: input.missingInputs.slice(0, 4).join("、"),
    });
  }
  const authors = uniqueOrdered(support.map((item) => item.author).filter(Boolean), 8);
  return {
    title: "本轮综合讲义",
    status: support.length > 0 ? "source_backed" : "needs_source",
    summary: support.length > 0
      ? "这是从旧学习档案恢复的保守讲义，已绑定旧档案中的已采纳来源。"
      : "这是从旧学习档案恢复的保守讲义；当前缺少已采纳来源，先用它重新检索和补证据。",
    sourceCoverage: {
      sourceCount: support.length,
      evidenceCount: support.length,
      authorCount: authors.length,
      authors,
    },
    sourceClaimIds: support.map((item) => item.claimId).filter(Boolean),
    points,
    authorPerspectives: [],
    conflicts: [],
    gaps,
    boundary: "系统综合不是作者原文证据；旧档案恢复的讲义只用于复习和重新追问，不能替代已采纳来源摘录。",
  };
}

function matchingSourceIndexForEvidence(evidence, sources = []) {
  const evidenceKeys = new Set(evidenceIdentityKeys(evidence));
  if (evidenceKeys.size === 0) return undefined;
  return (Array.isArray(sources) ? sources : []).findIndex((source) =>
    sourceIdentityKeys(source).some((key) => evidenceKeys.has(key)),
  );
}

function normalizeBusinessDecision(decision) {
  if (!decision || typeof decision !== "object") return undefined;
  const rows = (items, limit) => Array.isArray(items)
    ? items
        .map((item, index) => ({
          id: sanitizeText(item?.id || `business-data:${index}`, 80),
          label: sanitizeText(item?.label, 120),
          value: sanitizeText(item?.value, 180),
          why: sanitizeText(item?.why, 220),
          role: sanitizeText(item?.role, 40),
        }))
        .filter((item) => item.label)
        .slice(0, limit)
    : [];
  return {
    title: sanitizeText(decision.title || "当前产品判断", 100),
    status: ["ready", "insufficient_data", "needs_source", "needs_data"].includes(decision.status)
      ? decision.status
      : "needs_data",
    priority: sanitizeText(decision.priority || "insufficient", 80),
    label: sanitizeText(decision.label, 220),
    summary: sanitizeText(decision.summary, 320),
    supportingData: rows(decision.supportingData, 6),
    opposingData: rows(decision.opposingData, 4),
    missingData: rows(decision.missingData, 5),
    boundary: sanitizeText(decision.boundary || "用户产品数据不是作者原文证据，只用于判断适配性。", 320),
  };
}

function normalizeReviewState(state) {
  if (!state || typeof state !== "object") {
    return { checked: {}, updatedAt: "" };
  }
  const checked = state.checked && typeof state.checked === "object"
    ? Object.fromEntries(
        Object.entries(state.checked)
          .filter(([key, value]) => typeof key === "string" && value === true)
          .slice(0, 120)
          .map(([key]) => [sanitizeText(key, 140), true]),
      )
    : {};
  return {
    checked,
    updatedAt: sanitizeText(state.updatedAt, 40),
  };
}

function normalizeSelfTestState(state) {
  if (!state || typeof state !== "object") {
    return { mastered: {}, updatedAt: "" };
  }
  const mastered = state.mastered && typeof state.mastered === "object"
    ? Object.fromEntries(
        Object.entries(state.mastered)
          .filter(([key, value]) => typeof key === "string" && value === true)
          .slice(0, 120)
          .map(([key]) => [sanitizeText(key, 140), true]),
      )
    : {};
  return {
    mastered,
    updatedAt: sanitizeText(state.updatedAt, 40),
  };
}

function normalizeBusinessVerificationRecords(records) {
  if (!Array.isArray(records)) return [];
  return records
    .map((record, index) => {
      const rawText = sanitizeText(record?.rawText || record?.text, 1800);
      const createdAt = sanitizeText(record?.createdAt, 40);
      const id = sanitizeText(record?.id || `business:${stableHash(`${createdAt}:${rawText}:${index}`)}`, 90);
      const status = record?.status === "ready" ? "ready" : "needs_more_input";
      const sections = normalizeIntakeSections(record?.sections);
      const summary = sanitizeText(record?.summary, 220);
      if (!rawText && !summary && sections.every((section) => section.items.length === 0)) return null;
      return {
        id,
        createdAt,
        status,
        rawText,
        summary,
        sections,
        missing: sanitizeList(record?.missing, 10, 120),
        diagnosticPrompt: sanitizeText(record?.diagnosticPrompt, 1800),
        caution: sanitizeText(
          record?.caution || "这是用户提供的业务验证材料，不是本地资料证据；只保存在学习档案里，不写入原始知识库。",
          240,
        ),
      };
    })
    .filter(Boolean)
    .slice(0, 8);
}

function normalizeExperimentResultRecords(records) {
  if (!Array.isArray(records)) return [];
  return records
    .map((record, index) => {
      const rawText = sanitizeText(record?.rawText || record?.text, 1800);
      const createdAt = sanitizeText(record?.createdAt, 40);
      const id = sanitizeText(record?.id || `experiment:${stableHash(`${createdAt}:${rawText}:${index}`)}`, 90);
      const metrics = normalizeExperimentMetrics(record?.metrics);
      const outcome = ["positive", "negative", "mixed", "inconclusive"].includes(record?.outcome)
        ? record.outcome
        : classifyExperimentOutcome(metrics);
      if (!rawText && metrics.length === 0) return null;
      return {
        id,
        createdAt,
        rawText,
        outcome,
        metrics,
        missing: sanitizeList(record?.missing, 8, 120),
        summary: sanitizeText(record?.summary || experimentOutcomeSummary(outcome, metrics), 260),
        nextAction: sanitizeText(record?.nextAction || experimentNextAction(outcome), 260),
        caution: sanitizeText(
          record?.caution || "这是用户回填的实验复盘，不是作者原文证据，也不会写入原始知识库。",
          240,
        ),
      };
    })
    .filter(Boolean)
    .slice(0, 8);
}

function normalizeExperimentMetrics(metrics) {
  if (!Array.isArray(metrics)) return [];
  return metrics
    .map((metric) => {
      const name = sanitizeText(metric?.name, 24).toUpperCase();
      const before = Number(metric?.before);
      const after = Number(metric?.after);
      const direction = ["up", "down", "flat"].includes(metric?.direction) ? metric.direction : after > before ? "up" : after < before ? "down" : "flat";
      if (!name || !Number.isFinite(before) || !Number.isFinite(after)) return null;
      return {
        name,
        before,
        after,
        direction,
        favorable: metric?.favorable === true,
        unfavorable: metric?.unfavorable === true,
        label: sanitizeText(metric?.label || `${name} ${before}% -> ${after}%`, 80),
      };
    })
    .filter(Boolean)
    .slice(0, 12);
}

function normalizeIntakeSections(sections) {
  if (!Array.isArray(sections)) return [];
  return sections
    .map((section) => ({
      id: sanitizeText(section?.id, 40),
      label: sanitizeText(section?.label || "产品信息", 80),
      items: sanitizeList(section?.items, 8, 220),
      missing: sanitizeList(section?.missing, 4, 120),
    }))
    .filter((section) => section.id || section.label || section.items.length > 0)
    .slice(0, 8);
}

function normalizeDiagnosisPanel(panel) {
  if (!panel || typeof panel !== "object") return undefined;
  const tracks = Array.isArray(panel.tracks)
    ? panel.tracks
        .map((track, trackIndex) => {
          const trackId = sanitizeText(track?.id || `track:${trackIndex}`, 80);
          return {
            id: trackId,
            label: sanitizeText(track?.label || "检查项", 80),
            level: sanitizeText(track?.level, 40),
            why: sanitizeText(track?.why, 220),
            prompt: sanitizeText(track?.prompt, 420),
            checks: Array.isArray(track?.checks)
              ? track.checks
                  .map((check, checkIndex) => ({
                    id: sanitizeText(check?.id || `${trackId}:${checkIndex}`, 100),
                    label: sanitizeText(check?.label || check, 160),
                  }))
                  .filter((check) => check.label)
                  .slice(0, 6)
              : [],
          };
        })
        .filter((track) => track.label && track.checks.length > 0)
        .slice(0, 6)
    : [];
  if (tracks.length === 0) return undefined;

  const allowedCheckKeys = new Set(
    tracks.flatMap((track) => track.checks.map((check) => diagnosisCheckKey(track.id, check.id))),
  );
  const checked = panel.checked && typeof panel.checked === "object"
    ? Object.fromEntries(
        Object.entries(panel.checked)
          .filter(([key, value]) => value === true && allowedCheckKeys.has(key))
          .slice(0, 80)
          .map(([key]) => [sanitizeText(key, 120), true]),
      )
    : {};

  return {
    summary: sanitizeText(panel.summary, 220),
    priority: sanitizeText(panel.priority, 180),
    reason: sanitizeText(panel.reason, 280),
    tracks,
    checked,
    caution: sanitizeText(panel.caution || "诊断勾选代表用户排查进度，不是原文证据。", 260),
  };
}

function buildDossierDiagnosisSummary(dossier) {
  const panel = dossier.diagnosisPanel;
  if (!panel || !Array.isArray(panel.tracks) || panel.tracks.length === 0) return undefined;
  const totalChecks = panel.tracks.reduce((count, track) => count + (Array.isArray(track.checks) ? track.checks.length : 0), 0);
  const checkedChecks = Object.values(panel.checked || {}).filter((value) => value === true).length;
  return {
    summary: sanitizeText(panel.summary, 220),
    priority: sanitizeText(panel.priority, 180),
    reason: sanitizeText(panel.reason, 280),
    totalTracks: panel.tracks.length,
    totalChecks,
    checkedChecks,
    caution: sanitizeText(panel.caution, 260),
  };
}

function buildBusinessVerificationRecord(dossier, input = {}) {
  const rawText = sanitizeText(input.text, 1800);
  if (!rawText) return null;
  const createdAt = sanitizeText(input.createdAt || new Date().toISOString(), 40);
  const intake = buildProductIntake({ text: rawText }, dossier);
  const sections = normalizeIntakeSections(intake.sections);
  const usefulSections = sections.filter((section) => section.id !== "other" && section.items.length > 0);
  return {
    id: `business:${stableHash(`${createdAt}:${rawText}`)}`,
    createdAt,
    status: usefulSections.length > 0 ? "ready" : "needs_more_input",
    rawText,
    summary: sanitizeText(intake.summary, 220),
    sections,
    missing: sanitizeList(intake.missing, 10, 120),
    diagnosticPrompt: sanitizeText(intake.diagnosticPrompt, 1800),
    caution: "这是用户提供的业务验证材料，不是本地资料证据；只保存在学习档案里，不写入原始知识库。",
  };
}

function buildBusinessVerificationPanel(dossier) {
  const records = dossier.businessVerificationRecords.slice(0, 8);
  const readyRecords = records.filter((record) => record.status === "ready").length;
  const dimensions = buildBusinessVerificationDimensions(records);
  const readyDimensions = dimensions.filter((dimension) => dimension.status === "ready").length;
  const completeDimensions = dimensions.filter((dimension) => dimension.completeness === "complete").length;
  const coverage = {
    ready: readyDimensions,
    total: dimensions.length,
    percent: dimensions.length > 0 ? Math.round((readyDimensions / dimensions.length) * 100) : 0,
    complete: completeDimensions,
    completePercent: dimensions.length > 0 ? Math.round((completeDimensions / dimensions.length) * 100) : 0,
  };
  const nextDimension = dimensions.find((dimension) => dimension.status !== "ready" || dimension.completeness !== "complete") || null;
  const validationPlan = buildBusinessValidationPlan(dimensions, coverage, records);
  const experimentResults = buildExperimentResultsPanel(dossier.experimentResultRecords);
  return {
    summary: records.length > 0
      ? `这个档案保存了 ${records.length} 条业务验证记录，其中 ${readyRecords} 条已有可用于追问的具体材料；材料线索覆盖 ${coverage.ready}/${coverage.total}，材料完整 ${coverage.complete}/${coverage.total}。`
      : "还没有保存业务验证记录。粘贴你的产品材料后，可以把它和学习档案分开保存。",
    caution: "线索覆盖只代表用户材料涉及了哪些维度；材料完整度才表示关键材料是否补齐，但仍不代表产品验证完成，也不会把用户材料写入原始知识库。",
    totalRecords: records.length,
    readyRecords,
    coverage,
    dimensions,
    nextDimension,
    validationPlan,
    experimentResults,
    records,
  };
}

function buildDossierValidationPackProgress(dossier) {
  const pack = dossier.validationPack;
  if (!pack) return null;
  const sourceStatus = pack.status === "source_backed" ? "source_backed" : "needs_source";
  const materialCount = dossier.businessVerificationRecords.length;
  const experimentCount = dossier.experimentResultRecords.length;
  const status = sourceStatus === "needs_source"
    ? "needs_source"
    : experimentCount > 0
      ? "experiment_reviewed"
      : materialCount > 0
        ? "materials_ready"
        : "pending_materials";
  const statusLabel = validationProgressStatusLabel(status);
  const summary = validationProgressSummary(status, materialCount, experimentCount);
  return {
    title: pack.title || "验证任务包",
    status,
    statusLabel,
    sourceStatus,
    sourceStatusLabel: validationSourceStatusLabel(sourceStatus),
    summary,
    boundary: "任务包不是作者原文证据；业务材料和实验复盘只用于验证你的产品，不会改写原作者资料。",
    counts: {
      hypotheses: pack.hypotheses.length,
      dataRequests: pack.dataRequests.length,
      experiments: pack.experiments.length,
      decisionRules: pack.decisionRules.length,
      materialRecords: materialCount,
      experimentResults: experimentCount,
    },
    hypotheses: pack.hypotheses.slice(0, 3),
    dataRequests: pack.dataRequests.slice(0, 6),
    experiments: pack.experiments.slice(0, 3),
    decisionRules: pack.decisionRules.slice(0, 5),
    nextPrompt: pack.followUpPrompt || validationProgressFallbackPrompt(dossier),
  };
}

function buildDossierSynthesisGuide(dossier) {
  const synthesis = dossier.synthesisAnswer;
  if (!synthesis) return null;
  const points = synthesis.points.slice(0, 6).map((point) => ({
    id: point.id,
    label: point.label || "综合要点",
    text: point.text,
    basis: point.basis,
    supportCount: point.support.length,
    sourceIndexes: point.sourceIndexes.slice(0, 6),
    prompt: sanitizeText(`请围绕这个综合要点继续学习：${point.label || point.text || dossier.question}`, 360),
  }));
  const gaps = synthesis.gaps.slice(0, 6).map((item) => ({
    id: item.id,
    label: item.label || "待补缺口",
    reason: item.reason,
    prompt: sanitizeText(`请先补这个学习缺口：${item.label || item.reason || dossier.question}`, 360),
  }));
  return {
    title: synthesis.title || "本轮综合讲义",
    status: synthesis.status,
    statusLabel: synthesisStatusLabel(synthesis.status),
    summary: synthesis.summary || "这是根据本轮已定位来源整理出的学习讲义。",
    boundary: "系统综合不是作者原文证据；它只帮助复习和追问，不能替代已采纳来源摘录。",
    coverage: synthesis.sourceCoverage,
    points,
    gaps,
    nextPrompt: points[0]?.prompt || gaps[0]?.prompt || dossier.followUps[0] || dossier.question,
  };
}

function synthesisStatusLabel(status) {
  if (status === "source_backed") return "已有来源支撑";
  if (status === "needs_review") return "需要先复核";
  return "需要补来源";
}

function validationProgressStatusLabel(status) {
  if (status === "needs_source") return "先补作者来源";
  if (status === "pending_materials") return "待补材料";
  if (status === "materials_ready") return "已补材料";
  if (status === "experiment_reviewed") return "已复盘";
  return "待验证";
}

function validationSourceStatusLabel(status) {
  return status === "source_backed" ? "已有作者来源" : "缺少作者来源";
}

function validationProgressSummary(status, materialCount, experimentCount) {
  if (status === "needs_source") return "这轮任务包还缺作者来源，先补作者来源后再把业务数据用于判断。";
  if (status === "pending_materials") return "还没有补真实产品材料，先按任务包收集数据，再进入判断。";
  if (status === "materials_ready") return `已保存 ${materialCount} 条业务材料，下一步应按任务包做小实验或复盘。`;
  if (status === "experiment_reviewed") return `已保存 ${materialCount} 条业务材料，已回填 ${experimentCount} 条实验复盘，可以开始判断哪些动作有效。`;
  return "把任务包里的假设、数据和实验逐步补齐。";
}

function validationProgressFallbackPrompt(dossier) {
  return sanitizeText(
    `请基于这个学习档案继续推进验证任务包：${dossier.question || dossier.title || ""}`,
    420,
  );
}

function buildExperimentResultRecord(input = {}) {
  const rawText = sanitizeText(input.text, 1800);
  if (!rawText) return null;
  const createdAt = sanitizeText(input.createdAt || new Date().toISOString(), 40);
  const metrics = extractExperimentMetrics(rawText);
  const outcome = classifyExperimentOutcome(metrics);
  const missing = buildExperimentResultMissing(rawText, metrics);
  return {
    id: `experiment:${stableHash(`${createdAt}:${rawText}`)}`,
    createdAt,
    rawText,
    outcome,
    metrics,
    missing,
    summary: experimentOutcomeSummary(outcome, metrics),
    nextAction: experimentNextAction(outcome, missing),
    caution: "这是用户回填的实验复盘，不是作者原文证据，也不会写入原始知识库。",
  };
}

function buildExperimentResultsPanel(records = []) {
  const safeRecords = Array.isArray(records) ? records.slice(0, 8) : [];
  const latest = safeRecords[0] || null;
  const counts = safeRecords.reduce((acc, record) => {
    acc[record.outcome] = (acc[record.outcome] || 0) + 1;
    return acc;
  }, {});
  return {
    summary: {
      total: safeRecords.length,
      outcome: latest?.outcome || "none",
      message: latest
        ? `已保存 ${safeRecords.length} 条实验复盘，最近一次判断为：${experimentOutcomeLabel(latest.outcome)}。`
        : "还没有实验复盘。执行小实验后，把前后数据贴回来，系统会帮你判断下一步。",
      counts,
    },
    records: safeRecords,
    caution: "实验复盘只用于当前学习档案，不会变成作者资料证据。",
  };
}

function extractExperimentMetrics(text) {
  return ["CTR", "CVR", "ACOS"].map((name) => extractMetricChange(name, text)).filter(Boolean);
}

function extractMetricChange(name, text) {
  const pattern = new RegExp(`${name}\\s*(?:从|由)?\\s*(\\d+(?:\\.\\d+)?)\\s*%?\\s*(?:到|至|->|→|变成|提升到|降到)\\s*(\\d+(?:\\.\\d+)?)\\s*%?`, "i");
  const match = String(text || "").match(pattern);
  if (!match) return null;
  const before = Number(match[1]);
  const after = Number(match[2]);
  if (!Number.isFinite(before) || !Number.isFinite(after)) return null;
  const direction = after > before ? "up" : after < before ? "down" : "flat";
  const favorable = name === "ACOS" ? direction === "down" : direction === "up";
  const unfavorable = name === "ACOS" ? direction === "up" : direction === "down";
  return {
    name,
    before,
    after,
    direction,
    favorable,
    unfavorable,
    label: `${name} ${before}% -> ${after}%`,
  };
}

function classifyExperimentOutcome(metrics) {
  if (!Array.isArray(metrics) || metrics.length === 0) return "inconclusive";
  const favorable = metrics.filter((metric) => metric.favorable).length;
  const unfavorable = metrics.filter((metric) => metric.unfavorable).length;
  const complete = metrics.some((metric) => metric.name === "CTR")
    && metrics.some((metric) => metric.name === "CVR")
    && metrics.some((metric) => metric.name === "ACOS");
  if (favorable > 0 && unfavorable === 0) return complete ? "positive" : "partial_positive";
  if (unfavorable > 0 && favorable === 0) return complete ? "negative" : "partial_negative";
  if (favorable > 0 && unfavorable > 0) return "mixed";
  return "inconclusive";
}

function buildExperimentResultMissing(text, metrics) {
  const missing = [];
  const value = String(text || "").toLowerCase();
  if (!/天|周|日|小时|time window|window/.test(value)) missing.push("时间窗口");
  if (!Array.isArray(metrics) || metrics.length === 0) missing.push("前后数据");
  if (!metrics.some((metric) => metric.name === "CTR")) missing.push("CTR 前后变化");
  if (!metrics.some((metric) => metric.name === "CVR")) missing.push("CVR 前后变化");
  if (!metrics.some((metric) => metric.name === "ACOS")) missing.push("ACOS 前后变化");
  return uniqueOrdered(missing, 6);
}

function experimentOutcomeSummary(outcome, metrics) {
  const metricText = metrics.length ? metrics.map((metric) => metric.label).join("；") : "缺少可识别的前后指标";
  return `${experimentOutcomeLabel(outcome)}：${metricText}`;
}

function experimentOutcomeLabel(outcome) {
  if (outcome === "positive") return "正向信号";
  if (outcome === "partial_positive") return "局部正向，仍需补数据";
  if (outcome === "negative") return "负向信号";
  if (outcome === "partial_negative") return "局部负向，仍需补数据";
  if (outcome === "mixed") return "混合信号";
  return "证据不足";
}

function experimentNextAction(outcome, missing = []) {
  const missingText = Array.isArray(missing) && missing.length > 0 ? `先补齐 ${missing.slice(0, 3).join("、")}，` : "先补齐关键指标，";
  if (outcome === "positive") return "继续验证同一方向，复核时间窗口和流量来源后，再小范围扩大。";
  if (outcome === "negative") return "先不要扩大，回到关键词意图、主图表达和页面承接重新排查。";
  if (outcome === "partial_positive") return `${missingText}确认点击提升没有牺牲转化和广告效率，再决定是否继续同方向测试。`;
  if (outcome === "partial_negative") return `${missingText}先不要扩大，确认负向变化是否稳定存在。`;
  if (outcome === "mixed") return "拆分流量来源和页面承接，确认是点击质量问题还是转化承接问题。";
  return "补齐时间窗口、前后数据和流量来源，再重新判断。";
}

function buildBusinessVerificationDimensions(records) {
  return PRODUCT_INTAKE_SECTIONS
    .filter((section) => section.id !== "other")
    .map((section) => {
      const matchedRecords = [];
      const productTextParts = [];
      const latestItems = [];
      const missing = [];

      records.forEach((record) => {
        const recordSection = Array.isArray(record.sections)
          ? record.sections.find((item) => item.id === section.id)
          : null;
        const items = sanitizeList(recordSection?.items || [], 8, 180);
        const gaps = sanitizeList(recordSection?.missing || [], 6, 80);
        if (items.length > 0) {
          matchedRecords.push(record.id);
          productTextParts.push(record.rawText);
          latestItems.push(...items);
        }
        missing.push(...gaps);
      });

      const uniqueItems = uniqueOrdered(latestItems, 5);
      const uniqueMissing = uniqueItems.length > 0
        ? uniqueOrdered(missing, 5)
        : section.required.slice(0, 4);
      const status = uniqueItems.length > 0 ? "ready" : "missing";
      const completeness = status === "ready" && uniqueMissing.length === 0 ? "complete" : status === "ready" ? "partial" : "missing";
      const productText = sanitizeText((productTextParts.length > 0 ? productTextParts : records.map((record) => record.rawText)).join("\n"), 1800);

      return {
        id: section.id,
        label: section.label,
        status,
        completeness,
        records: matchedRecords.length,
        latestItems: uniqueItems,
        missing: uniqueMissing,
        productText,
        prompt: buildBusinessDimensionPrompt(section, status, uniqueItems, uniqueMissing),
      };
    });
}

function buildBusinessDimensionPrompt(section, status, items, missing) {
  const base = `请基于这个学习档案，单独检查「${section.label}」这个验证维度。`;
  if (status === "ready") {
    return sanitizeText(`${base} 我已经补充的材料：${items.join("；")}。${BUSINESS_PROMPT_EVIDENCE_BOUNDARY} 请告诉我这个维度下一步最应该验证什么。`, 900);
  }
  return sanitizeText(`${base} 现在还缺：${missing.join("、")}。${BUSINESS_PROMPT_EVIDENCE_BOUNDARY} 请告诉我应该先补哪类材料，以及补完后如何判断。`, 900);
}

function buildBusinessValidationPlan(dimensions, coverage, records) {
  if (records.length === 0) {
    return {
      summary: "先保存真实产品材料，再生成补料清单、判断信号和小实验。",
      priorityDimension: null,
      materialChecklist: [],
      decisionGates: [],
      experiments: [],
      coverage,
      caution: "没有业务验证记录时，系统不会生成判断门槛或实验建议。",
    };
  }
  const priorityDimension = dimensions.find((dimension) => dimension.completeness !== "complete") || dimensions[0] || null;
  const materialChecklist = dimensions.flatMap((dimension) => buildDimensionMaterialChecklist(dimension)).slice(0, 12);
  const decisionGates = dimensions.flatMap((dimension) => buildDimensionDecisionGates(dimension)).slice(0, 10);
  const experiments = dimensions
    .filter((dimension) => dimension.status === "ready" || dimension.id === priorityDimension?.id)
    .map((dimension) => buildDimensionExperiment(dimension))
    .filter(Boolean)
    .slice(0, 5);

  return {
    summary: records.length > 0
      ? `当前第一个待补维度是「${priorityDimension?.label || "产品材料"}」；先补缺口，再用小实验验证，而不是直接下结论。`
      : "先补真实产品材料，再生成验证方案。",
    priorityDimension: priorityDimension
      ? {
          id: priorityDimension.id,
          label: priorityDimension.label,
          reason: priorityReason(priorityDimension),
          prompt: sanitizeText(`${priorityDimension.prompt} 请把回答压成补料清单、待验证判断信号和 1 个小实验。`, 1100),
        }
      : null,
    materialChecklist,
    decisionGates,
    experiments,
    coverage,
    caution: "这是基于用户材料生成的验证方案，不是作者原文证据，也不是结论；执行后仍要回填真实数据继续判断。",
  };
}

function buildDimensionMaterialChecklist(dimension) {
  const missing = dimension.missing.length > 0 ? dimension.missing : ["补充能验证这一维度的截图或数据"];
  return missing.slice(0, 3).map((item) => ({
    id: `material:${dimension.id}:${stableHash(item)}`,
    dimensionId: dimension.id,
    dimensionLabel: dimension.label,
    label: sanitizeText(item, 120),
    reason: materialReason(dimension, item),
    prompt: sanitizeText(`我准备补充「${dimension.label}」材料：${item}。请告诉我应该怎么截图、记录或对比，才对下一步判断有用。`, 520),
  }));
}

function buildDimensionDecisionGates(dimension) {
  const gates = BUSINESS_DIMENSION_GATES[dimension.id] || [];
  return gates.map((gate) => ({
    id: `gate:${dimension.id}:${stableHash(gate.metric)}`,
    dimensionId: dimension.id,
    dimensionLabel: dimension.label,
    metric: gate.metric,
    pass: gate.pass,
    fail: gate.fail,
    source: "业务操作假设",
    boundary: "这是系统整理的业务操作假设，不是作者原文，也不是通用硬标准；需要结合类目、时间窗口和真实数据复核。",
  }));
}

function buildDimensionExperiment(dimension) {
  const experiment = BUSINESS_DIMENSION_EXPERIMENTS[dimension.id] || BUSINESS_DIMENSION_EXPERIMENTS.other;
  if (!experiment) return null;
  const missingHint = dimension.missing.length > 0 ? `当前仍缺：${dimension.missing.slice(0, 3).join("、")}。` : "";
  return {
    id: `experiment:${dimension.id}`,
    dimensionId: dimension.id,
    dimensionLabel: dimension.label,
    title: experiment.title,
    hypothesis: experiment.hypothesis,
    steps: experiment.steps,
    successSignal: experiment.successSignal,
    prompt: sanitizeText(`${missingHint}${BUSINESS_PROMPT_EVIDENCE_BOUNDARY} 请把「${dimension.label}」设计成一个小实验：${experiment.title}，并告诉我记录什么数据。`, 980),
  };
}

function priorityReason(dimension) {
  if (dimension.status === "missing") return `这个维度还没有任何可用线索，先补 ${dimension.missing.slice(0, 2).join("、") || "具体材料"}。`;
  return `这个维度已有线索但还不完整，先补 ${dimension.missing.slice(0, 2).join("、") || "关键证据"}，避免过早下判断。`;
}

function materialReason(dimension, item) {
  if (dimension.status === "missing") return `缺少 ${item}，系统只能给方向，无法判断 ${dimension.label} 是否真的成立。`;
  return `已有 ${dimension.label} 线索，但还缺 ${item}，需要补齐后再做下一步实验。`;
}

const BUSINESS_DIMENSION_GATES = {
  visual: [
    { metric: "主图点击入口", pass: "主图能在同关键词前 3-5 个竞品里一眼表达差异", fail: "主图仍像通用白底图，卖点和竞品不可区分" },
    { metric: "主图/副图一致性", pass: "副图能承接主图承诺并解释使用场景", fail: "点击后页面没有继续解释主图卖点" },
  ],
  metrics: [
    { metric: "CTR", pass: "主图或标题调整后同流量来源下 CTR 上升", fail: "CTR 无变化或下降，优先复查关键词和视觉匹配" },
    { metric: "CVR", pass: "点击质量稳定时 CVR 不下降", fail: "CTR 上升但 CVR 下降，说明吸引了不匹配流量" },
  ],
  listing: [
    { metric: "页面承接", pass: "标题、五点、A+ 能解释主图承诺", fail: "页面信息和主图卖点断裂" },
    { metric: "信任门槛", pass: "价格、评价、评分没有明显拖累转化", fail: "视觉改动前应先处理价格或评价劣势" },
  ],
  ads: [
    { metric: "广告验证", pass: "同关键词、同预算下点击或转化信号改善", fail: "ACOS 持续偏高且点击无改善，先停放大预算" },
    { metric: "流量拆分", pass: "SP/SBV/自然流量分开观察", fail: "混在一起看，无法判断视觉还是流量问题" },
  ],
  keywords: [
    { metric: "关键词匹配", pass: "主图卖点和核心关键词用户意图一致", fail: "关键词意图与主图表达不一致" },
    { metric: "竞品对照", pass: "至少 3 个核心竞品能说明同质化或差异机会", fail: "没有竞品对照，不能判断差异化是否真实" },
  ],
};

const BUSINESS_DIMENSION_EXPERIMENTS = {
  visual: {
    title: "主图 A/B 小实验",
    hypothesis: "如果点击低主要来自视觉入口，新主图应先改善 CTR，且 CVR 不明显下降。",
    steps: ["保留同一关键词和广告结构", "替换一版突出差异卖点的主图", "连续观察同时间窗口 CTR、CVR、ACOS"],
    successSignal: "CTR 上升且 CVR 不下降，再扩大到副图和页面承接。",
  },
  metrics: {
    title: "同窗口数据复核",
    hypothesis: "只有在同流量来源和同时间窗口下，CTR/CVR 才能说明改动方向。",
    steps: ["固定时间窗口", "拆分自然、SP、SBV 数据", "记录曝光、点击、转化、ACOS"],
    successSignal: "指标变化能对应到具体入口，而不是全渠道混合波动。",
  },
  listing: {
    title: "页面承接复核",
    hypothesis: "如果主图承诺和页面信息一致，点击后的转化阻力会降低。",
    steps: ["列出主图承诺", "逐项检查标题、五点、副图、A+", "标出没有承接的卖点"],
    successSignal: "页面能连续解释核心卖点，且用户疑问减少。",
  },
  ads: {
    title: "低风险广告验证",
    hypothesis: "先用小预算验证入口，不用大预算放大不确定问题。",
    steps: ["选择 1-2 个核心词", "保持预算和竞价稳定", "只观察入口改动前后的点击和转化"],
    successSignal: "点击或转化改善后再增加预算。",
  },
  keywords: {
    title: "关键词意图对照",
    hypothesis: "关键词用户意图和视觉卖点一致时，点击质量更稳定。",
    steps: ["选 3 个核心搜索词", "截取搜索结果页前 10 个竞品", "标注用户最可能比较的卖点"],
    successSignal: "能明确主图应该表达的第一卖点。",
  },
  other: {
    title: "最小补料实验",
    hypothesis: "先补最缺材料，再决定是否继续投入。",
    steps: ["列出当前缺口", "补一类最关键材料", "回到学习档案重新追问"],
    successSignal: "下一轮问题能从泛泛建议变成具体判断。",
  },
};

function uniqueOrdered(items, limit) {
  const seen = new Set();
  const values = [];
  for (const item of items) {
    const value = sanitizeText(item, 220);
    if (!value || seen.has(value)) continue;
    seen.add(value);
    values.push(value);
    if (values.length >= limit) break;
  }
  return values;
}

function buildReviewQueue(dossier) {
  const items = [];
  const checked = dossier.reviewState?.checked || {};
  const addItem = (kind, label, reason, prompt, identity = "") => {
    const safeLabel = sanitizeText(label, 180);
    if (!safeLabel) return;
    const id = reviewItemId(kind, identity || safeLabel);
    if (items.some((item) => item.id === id || item.label === safeLabel)) return;
    const completion = reviewItemCompletion(dossier, {
      id,
      kind,
      label: safeLabel,
      identity,
      checked: checked[id] === true,
    });
    items.push({
      id,
      kind,
      label: safeLabel,
      reason: sanitizeText(reason, 220),
      prompt: sanitizeText(prompt, 520),
      done: completion.done,
      canManualComplete: completion.canManualComplete,
      completionLabel: completion.label,
      completionReason: completion.reason,
    });
  };

  const panel = dossier.diagnosisPanel;
  if (panel?.tracks?.length) {
    panel.tracks.slice(0, 6).forEach((track) => {
      (track.checks || []).slice(0, 6).forEach((check) => {
        addItem(
          "diagnosis",
          `${track.label}：${check.label}`,
          track.why || panel.reason || "来自保存时的诊断排查面板。",
          track.prompt || `请基于这个学习档案继续排查：${track.label} - ${check.label}`,
          `${track.id}:${check.id}`,
        );
      });
    });
  }

  dossier.nextActions.slice(0, 8).forEach((item) => {
    addItem(
      "action",
      item,
      dossier.takeaway || "来自学习档案的行动顺序。",
      `我正在复盘“${item}”。请基于这个学习档案判断下一步应该验证什么。`,
      item,
    );
  });

  const missingInputs = [...new Set([
    ...dossier.missingInputs,
    ...(dossier.productInputSummary?.missing || []),
  ])].slice(0, 8);
  missingInputs.forEach((item) => {
    addItem(
      "input",
      `补充：${item}`,
      "这个信息会让下一轮判断更贴近你的真实产品。",
      `我补充了${item}：。请基于这个学习档案重新诊断，并告诉我先看什么。`,
      item,
    );
  });

  dossier.acceptedEvidence.slice(0, 5).forEach((item) => {
    addItem(
      "evidence",
      item.title ? `复核证据：${item.title}` : "复核已采纳证据",
      item.text || item.quote || "来自已采纳的原文证据快照。",
      `请用这条已采纳证据复核我的页面判断：${item.text || item.quote || ""}`,
      item.sourceKey || item.sourcePath || item.sourceUrl || item.title,
    );
  });

  dossier.followUps.slice(0, 4).forEach((item) => {
    addItem("question", `追问：${item}`, "来自学习卡片的后续问题。", item, item);
  });

  const safeItems = items.slice(0, 24);
  const total = safeItems.length;
  const completed = safeItems.filter((item) => item.done).length;
  const nextItem = safeItems.find((item) => !item.done) || null;
  return {
    summary: total > 0
      ? `这个档案生成了 ${total} 个推进项，已处理 ${completed} 个。`
      : "这个档案还没有足够信息生成推进队列。",
    progress: {
      completed,
      total,
      percent: total > 0 ? Math.round((completed / total) * 100) : 0,
    },
    stages: ["diagnosis", "action", "input", "evidence", "question"].map((kind) => ({
      kind,
      total: safeItems.filter((item) => item.kind === kind).length,
      completed: safeItems.filter((item) => item.kind === kind && item.done).length,
    })).filter((stage) => stage.total > 0),
    nextItem,
    items: safeItems,
  };
}

function reviewItemCompletion(dossier, item) {
  if (item.kind === "input") {
    const matchedRecord = dossier.businessVerificationRecords.find((record) => businessRecordMentionsReviewInput(record, item.label, item.identity));
    return {
      done: Boolean(matchedRecord),
      canManualComplete: false,
      label: matchedRecord ? "已补材料" : "需先补材料",
      reason: matchedRecord
        ? "这个缺口已经有用户保存的产品材料记录。"
        : "这个缺口不能只靠勾选完成，需要先在档案里保存真实产品材料。",
    };
  }

  return {
    done: item.checked === true,
    canManualComplete: true,
    label: item.checked === true ? "已人工复盘" : "可人工复盘",
    reason: "勾选只代表你已经复盘，不会改变原文证据或产品材料。",
  };
}

function businessRecordMentionsReviewInput(record, label, identity) {
  const target = sanitizeText(String(identity || label || "").replace(/^补充：/, ""), 80);
  if (!target) return false;
  const haystack = sanitizeText(
    [
      record?.rawText,
      record?.summary,
      ...(Array.isArray(record?.sections) ? record.sections.flatMap((section) => section.items || []) : []),
    ].join("\n"),
    2000,
  );
  const normalizedHaystack = haystack.toLowerCase();
  const candidates = [target, ...reviewInputAliases(target)]
    .map((item) => sanitizeText(item, 80).toLowerCase())
    .filter(Boolean);
  return candidates.some((candidate) => normalizedHaystack.includes(candidate));
}

function reviewInputAliases(target) {
  const aliases = [];
  if (/链接|网址|url/i.test(target)) aliases.push("http://", "https://", "www.", "链接");
  if (/acos|广告/i.test(target)) aliases.push("acos", "广告");
  if (/流量|来源/i.test(target)) aliases.push("流量", "来源", "sp", "sbv", "自然");
  if (/竞品|对照/i.test(target)) aliases.push("竞品", "asin", "对照");
  return aliases;
}

function buildSelfTest(dossier) {
  const items = [];
  const mastered = dossier.selfTestState?.mastered || {};
  const addCard = (kind, question, answer, explanation, identity = "") => {
    const safeQuestion = sanitizeText(question, 220);
    const safeAnswer = sanitizeText(answer, 360);
    if (!safeQuestion || !safeAnswer) return;
    const id = selfTestItemId(kind, identity || `${safeQuestion}:${safeAnswer}`);
    if (items.some((item) => item.id === id || item.question === safeQuestion)) return;
    items.push({
      id,
      kind,
      question: safeQuestion,
      answer: safeAnswer,
      explanation: sanitizeText(explanation, 360) || "来自当前学习档案。",
      mastered: mastered[id] === true,
    });
  };

  if (dossier.takeaway) {
    addCard(
      "takeaway",
      "这个档案最核心的一句话结论是什么？",
      dossier.takeaway,
      "答案来自保存时的学习卡片结论，不是新增资料。",
      "takeaway",
    );
  }

  if (dossier.diagnosisPanel?.priority) {
    addCard(
      "diagnosis",
      "如果现在只先检查一个方向，保存时的诊断优先级是什么？",
      dossier.diagnosisPanel.priority,
      dossier.diagnosisPanel.reason || "答案来自保存时的诊断排查面板。",
      `diagnosis:${dossier.diagnosisPanel.priority}`,
    );
  }

  dossier.nextActions.slice(0, 3).forEach((item, index) => {
    addCard(
      "action",
      `行动顺序第 ${index + 1} 步是什么？`,
      item,
      "答案来自学习档案里的行动顺序。",
      `action:${index}:${item}`,
    );
  });

  dossier.missingInputs.slice(0, 3).forEach((item, index) => {
    addCard(
      "input",
      "下一轮判断前应该补充什么信息？",
      item,
      "答案来自学习档案里的缺口清单。",
      `input:${index}:${item}`,
    );
  });

  dossier.acceptedEvidence.slice(0, 3).forEach((item, index) => {
    const source = [item.author, item.title].filter(Boolean).join("《") + (item.title ? "》" : "");
    addCard(
      "evidence",
      "哪条原文证据可以支撑这个档案？",
      item.quote || item.text,
      source ? `原文来源：${source}` : "答案来自已采纳的原文证据快照。",
      item.sourceKey || item.sourcePath || item.sourceUrl || `evidence:${index}`,
    );
  });

  dossier.followUps.slice(0, 2).forEach((item, index) => {
    addCard(
      "question",
      "理解后可以继续追问什么？",
      item,
      "答案来自学习卡片里的后续追问。",
      `question:${index}:${item}`,
    );
  });

  const safeItems = items.slice(0, 10);
  const total = safeItems.length;
  const masteredCount = safeItems.filter((item) => item.mastered).length;
  return {
    summary: total > 0
      ? `这个档案生成了 ${total} 张档案回忆卡，已理解 ${masteredCount} 张。`
      : "这个档案还没有足够信息生成理解自测。",
    progress: {
      mastered: masteredCount,
      total,
      percent: total > 0 ? Math.round((masteredCount / total) * 100) : 0,
    },
    items: safeItems,
  };
}

function reviewItemId(kind, identity) {
  return `${kind}:${stableHash(`${kind}:${identity}`)}`;
}

function selfTestItemId(kind, identity) {
  return `${kind}:${stableHash(`self-test:${kind}:${identity}`)}`;
}

function reviewKindPriority(kind) {
  if (kind === "diagnosis") return 1;
  if (kind === "action") return 2;
  if (kind === "input") return 3;
  if (kind === "evidence") return 4;
  if (kind === "question") return 5;
  return 9;
}

function stableHash(value) {
  let hash = 5381;
  for (const char of String(value || "")) {
    hash = ((hash << 5) + hash) ^ char.codePointAt(0);
  }
  return (hash >>> 0).toString(36);
}

function diagnosisCheckKey(trackId, checkId) {
  return `${trackId}::${checkId}`;
}

function sourceIdentityKeys(source) {
  return [source.sourcePath, source.sourceUrl, sourceKeyForSource(source)].filter(Boolean);
}

function evidenceIdentityKeys(evidence) {
  return [evidence?.sourceKey, evidence?.sourcePath, evidence?.sourceUrl, sourceKeyForSource(evidence || {})].filter(Boolean);
}

function sourceKeyForSource(source) {
  const direct = source.sourcePath || source.sourceUrl;
  if (direct) return direct;
  return [source.author, source.date, source.title].filter(Boolean).join("|");
}

function sourceLabel(source) {
  return [source.author, source.date, source.title].filter(Boolean).join(" · ");
}

function normalizeSourceKeys(keys) {
  if (!Array.isArray(keys)) return [];
  return [...new Set(keys.filter((key) => typeof key === "string" && key.trim()).map((key) => key.trim()))].slice(0, 100);
}

function normalizeAuthorNames(authors) {
  if (!Array.isArray(authors)) return [];
  return [...new Set(authors.map((author) => sanitizeText(author, 80)).filter(Boolean))].slice(0, 20);
}

function compactSourceKey(key) {
  const text = String(key || "");
  if (text.includes("|")) return text.split("|").filter(Boolean).slice(-1)[0] || text;
  const parts = text.split("/");
  return parts[parts.length - 1] || text;
}

function sanitizeList(items, limit, maxLength) {
  if (!Array.isArray(items)) return [];
  return items.map((item) => sanitizeText(item, maxLength)).filter(Boolean).slice(0, limit);
}

function sanitizeText(value, maxLength) {
  const text = String(value || "").replace(/\s+/g, " ").trim();
  return text.length > maxLength ? text.slice(0, maxLength) : text;
}

function firstMeaningfulLine(text) {
  return String(text || "")
    .split("\n")
    .map((line) => stripInlineMarkers(line).trim())
    .find((line) => line && !line.startsWith("问题：")) || "";
}

function inferQuestion(text) {
  const match = String(text || "").match(/问题：(.+)/);
  return match ? match[1] : "";
}

function stripInlineMarkers(text) {
  return String(text || "").replace(/【(?:(?:资料|证据|来源|推断|行动)\d+|缺少来源)】/g, "");
}
