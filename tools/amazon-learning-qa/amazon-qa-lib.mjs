import { buildProductIntake } from "./amazon-dossier-lib.mjs";

export const AMAZON_AUTHORS = ["张子卿", "飞翔的波波", "跨境电商长期主义"];
export const USER_SOURCE_AUTHOR = "我的资料";
const AUTHORS = [...AMAZON_AUTHORS, USER_SOURCE_AUTHOR];
const AUTHOR_SET = new Set(AMAZON_AUTHORS);
const GENERIC_CJK_TERMS = new Set([
  "如何",
  "怎么",
  "什么",
  "是否",
  "应该",
  "判断",
  "值得",
  "值不",
  "不值",
  "得做",
  "一个",
  "问题",
  "注意",
  "需要",
  "可以",
]);

const DOMAIN_EXPANSIONS = [
  {
    pattern: /选品|值不值得做|值得做|能不能做|做不做/,
    terms: ["选品", "新品", "产品", "市场", "容量", "关键词", "月销量", "销量", "竞争", "利润", "退货", "评分", "差异化"],
  },
  {
    pattern: /产品|卖不动|卖不出去|销量|价格|评价|竞品|类目|利润|库存|运营|店铺|fba/i,
    terms: ["产品", "销量", "价格", "评价", "竞品", "类目", "利润", "库存", "流量", "页面", "关键词", "广告", "转化", "fba"],
  },
  {
    pattern: /主图|图片|视觉|点击率|转化率/,
    terms: ["主图", "图片", "视觉", "点击率", "转化率", "文案", "页面", "对比", "流量"],
  },
  {
    pattern: /广告|推广|投放|acos|cpc/i,
    terms: ["广告", "推广", "投放", "关键词", "流量", "竞价", "预算", "转化", "自然排名", "acos", "cpc"],
  },
  {
    pattern: /listing|文案|关键词|收录|标题|search term/i,
    terms: ["listing", "文案", "关键词", "收录", "标题", "search term", "卖点", "五点", "bullet", "排名"],
  },
];

const GRAPH_CONCEPTS = [
  { label: "主图", pattern: /主图|首图|main image/i },
  { label: "点击率", pattern: /点击率|ctr/i },
  { label: "转化率", pattern: /转化率|转化|conversion/i },
  { label: "Listing", pattern: /listing|页面|商品页面/i },
  { label: "广告", pattern: /广告|推广|投放|acos|cpc/i },
  { label: "关键词", pattern: /关键词|词库|搜索词|search term/i },
  { label: "副图", pattern: /副图|附图|图片体系/i },
  { label: "A+", pattern: /a\\+|A\\+|品牌内容/i },
  { label: "评价", pattern: /评价|评分|review/i },
  { label: "流量", pattern: /流量|渠道|相关性/i },
  { label: "新品", pattern: /新品|新产品/i },
  { label: "选品", pattern: /选品|市场容量|竞争格局/i },
  { label: "文案", pattern: /文案|标题|五点|bullet/i },
  { label: "差异化", pattern: /差异化|不一样|独特|对比/i },
];

export const DEFAULT_SUGGESTED_QUESTIONS = [
  "新品选品应该如何判断是否值得做？",
  "Listing 文案关键词布局收录应该怎么做？",
  "主图视觉点击率转化率怎么优化？",
  "新品广告推广应该注意什么？",
  "一个产品卖不动时应该先检查什么？",
];

export function isAuthorComparisonRequest(text) {
  const value = String(text || "");
  return /对比|比较|不同作者|三位作者|三个作者|三大作者|张子卿|飞翔的波波|长期主义|作者.*观点|观点.*作者/.test(value);
}

export function buildKnowledgeHealthSummary(metrics = {}) {
  const documents = Number(metrics.documents || 0);
  const chunks = Number(metrics.chunks || 0);
  const embeddedChunks = Number(metrics.embeddedChunks || 0);
  const graphRelations = Number(metrics.graphRelations || 0);
  const sourceTree = metrics.sourceTree && typeof metrics.sourceTree === "object" ? metrics.sourceTree : undefined;
  const vectorCoveragePercent = chunks > 0 ? Math.round((embeddedChunks / chunks) * 1000) / 10 : 0;

  let level = "ok";
  let message = "知识库基础状态正常。";
  if (chunks > 0 && embeddedChunks === 0) {
    level = "needs_index";
    message = "语义索引还没有建立，现在主要依赖关键词匹配。";
  } else if (chunks > 0 && vectorCoveragePercent < 80) {
    level = "partial_index";
    message = "语义索引覆盖不完整，部分问题可能召回不稳定。";
  }

  if (graphRelations < 10) {
    message += " 知识关系图也很薄，暂时不能依赖图谱推理。";
  }

  if (sourceTree?.message) {
    message += ` ${sourceTree.message}`;
  }

  return {
    documents,
    chunks,
    embeddedChunks,
    graphRelations,
    vectorCoveragePercent,
    sourceTree,
    level,
    message,
  };
}

export function buildKnowledgeReadinessSummary(status = {}) {
  const health = status.health && typeof status.health === "object" ? status.health : {};
  const sourceTree = health.sourceTree && typeof health.sourceTree === "object" ? health.sourceTree : {};
  const manifestDocuments = Number(status.manifestDocuments || sourceTree.manifestDocuments || health.documents || 0);
  const storedDocuments = Number(status.storedDocuments || health.documents || 0);
  const chunks = Number(health.chunks || 0);
  const embeddedChunks = Number(health.embeddedChunks || 0);
  const vectorCoveragePercent = Number(health.vectorCoveragePercent || 0);
  const sourceTreeDocuments = Number(sourceTree.ingestedDocuments || 0);
  const sourceTreeTotal = Number(sourceTree.manifestDocuments || manifestDocuments || 0);
  const sourceTreeCoverage = Number(sourceTree.coveragePercent || 0);
  const sourceTreeQueued = Number(sourceTree.queuedJobs || 0);
  const sourceTreeFailed = Number(sourceTree.failedJobs || 0);
  const sourceTreeTrees = Number(sourceTree.trees || 0);
  const sourceTreeSummaries = Number(sourceTree.summaries || 0);

  const searchStatus = chunks > 0 && embeddedChunks > 0
    ? vectorCoveragePercent >= 99
      ? "ready"
      : "partial"
    : "missing";
  const citationStatus = manifestDocuments > 0 && storedDocuments >= manifestDocuments && chunks > 0
    ? "ready"
    : storedDocuments > 0 && chunks > 0
      ? "partial"
      : "missing";
  const learningStatus = sourceTreeFailed > 0
    ? "needs_attention"
    : sourceTreeTotal > 0 && sourceTreeDocuments >= sourceTreeTotal && sourceTreeQueued === 0 && sourceTreeTrees > 0 && sourceTreeSummaries > 0
      ? "ready"
      : sourceTreeDocuments > 0 || sourceTreeQueued > 0
        ? "processing"
        : "missing";

  const stages = [
    {
      id: "search",
      label: "可搜索",
      status: searchStatus,
      detail: searchStatus === "ready"
        ? `${formatReadinessNumber(embeddedChunks)}/${formatReadinessNumber(chunks)} 个语义片段已可检索。`
        : searchStatus === "partial"
          ? `${formatReadinessNumber(embeddedChunks)}/${formatReadinessNumber(chunks)} 个语义片段可检索，仍需补齐。`
          : "语义检索还没有建立完成。",
    },
    {
      id: "citation",
      label: "可引用",
      status: citationStatus,
      detail: citationStatus === "ready"
        ? `${formatReadinessNumber(storedDocuments)}/${formatReadinessNumber(manifestDocuments)} 篇资料已进入本地原文库，可回到来源核对。`
        : citationStatus === "partial"
          ? `${formatReadinessNumber(storedDocuments)}/${formatReadinessNumber(manifestDocuments)} 篇资料可引用，仍有原文未完成。`
          : "本地原文库还不足，不能稳定引用来源。",
    },
    {
      id: "learning",
      label: "可学习",
      status: learningStatus,
      detail: learningStatus === "ready"
        ? `OpenHuman 来源树已完成，${formatReadinessNumber(sourceTreeTrees)} 棵树、${formatReadinessNumber(sourceTreeSummaries)} 个摘要可用于结构化学习。`
        : learningStatus === "processing"
          ? `OpenHuman 来源树 ${formatReadinessNumber(sourceTreeDocuments)}/${formatReadinessNumber(sourceTreeTotal)} 篇，${formatReadinessNumber(sourceTreeQueued)} 个任务处理中；还不是完整可学习状态。`
          : learningStatus === "needs_attention"
            ? `OpenHuman 来源树有 ${formatReadinessNumber(sourceTreeFailed)} 个任务失败，需要处理后才能稳定学习。`
            : "OpenHuman 来源树还没有建立，当前只能搜索和引用，不能称为完整学习系统。",
    },
  ];

  const answerStatus = searchStatus === "ready" && citationStatus === "ready"
    ? "ready"
    : searchStatus === "ready" || citationStatus === "ready"
      ? "partial"
      : "missing";
  const level = answerStatus === "ready" && learningStatus === "ready"
    ? "ready"
    : answerStatus === "ready" && learningStatus === "needs_attention"
      ? "answer_ready_learning_attention"
      : answerStatus === "ready" && learningStatus === "processing"
        ? "answer_ready_learning_processing"
        : answerStatus === "ready"
          ? "searchable_citable"
          : searchStatus === "ready"
            ? "searchable"
            : "not_ready";
  const message = readinessMessage(stages);

  return {
    level,
    message,
    stages,
    answerStatus,
    searchStatus,
    citationStatus,
    learningStatus,
    sourceTreeCoverage,
  };
}

function readinessMessage(stages = []) {
  const byId = new Map(stages.map((stage) => [stage.id, stage]));
  const search = byId.get("search")?.status;
  const citation = byId.get("citation")?.status;
  const learning = byId.get("learning")?.status;
  if (search === "ready" && citation === "ready" && learning === "ready") {
    return "可搜索、可引用、可学习都已就绪，可以作为结构化学习系统使用。";
  }
  if (search === "ready" && citation === "ready" && learning === "processing") {
    return "可问答和可引用已就绪；OpenHuman 来源树仍在后台深加工，学习增强会继续补齐。";
  }
  if (search === "ready" && citation === "ready") {
    return "可搜索和可引用已就绪；OpenHuman 来源树未完成，暂时不能称为完整可学习系统。";
  }
  if (search === "ready") {
    return "可搜索已就绪，但来源引用和结构化学习还需要继续补齐。";
  }
  return "知识库还没有达到稳定可用状态，需要先补齐索引和来源。";
}

function formatReadinessNumber(value) {
  const number = Number(value || 0);
  return Number.isFinite(number) ? String(Math.trunc(number)) : "0";
}

export function parseOpenHumanContext(contextText) {
  if (!contextText || typeof contextText !== "string") return [];

  const matches = [...contextText.matchAll(/^(.+?)\s+(\d{4}-\d{2}-\d{2})\s+(.+?):\s+#\s+(.+)$/gm)]
    .filter((match) => AUTHORS.includes(match[1].trim()))
    .map((match) => ({
      index: match.index,
      author: match[1].trim(),
      date: match[2].trim(),
      title: (match[4] || match[3]).trim(),
    }));

  if (matches.length === 0) return parseMarkdownArticleBlocks(contextText);

  return matches.map((match, index) => {
    const next = matches[index + 1];
    const block = contextText.slice(match.index, next ? next.index : contextText.length).trim();
    if (isCandidateArticleBlock(block)) return null;
    const sourceUrl = lineValue(block, "原文链接");
    const sourcePath = lineValue(block, "来源文件");
    const body = cleanArticleBody(block);

    return {
      author: match.author,
      date: match.date,
      title: match.title,
      sourceUrl,
      sourcePath,
      sourceType: sourceMaterialKind({ author: match.author, sourceUrl, sourcePath }),
      excerpt: body.slice(0, 700),
      body,
    };
  }).filter(Boolean);
}

export function scoreSentences(question, articles, limit = 6) {
  const profile = buildQuestionProfile(question);
  const rows = [];

  for (const article of articles) {
    for (const sentence of splitSentences(article.body)) {
      const score = scoreText(sentence, profile);
      if (score <= 0) continue;
      rows.push({ text: sentence, score, source: article });
    }
  }

  rows.sort((a, b) => b.score - a.score || b.text.length - a.text.length);
  return dedupeSentences(rows).slice(0, limit);
}

export function buildQaPayload(question, contextText, retrievalQuestion = question, options = {}) {
  const normalizedContext = normalizeContextText(contextText);
  const excludedSourceKeys = normalizeSourceKeySet(options.excludedSourceKeys);
  const allowedAuthors = normalizeAuthorSet(options.allowedAuthors);
  const allowedSourceKeys = normalizeSourceKeySet(options.allowedSourceKeys);
  const allowedSourceCount = Number.isFinite(Number(options.allowedSourceCount)) ? Math.max(0, Math.floor(Number(options.allowedSourceCount))) : undefined;
  const rawArticles = parseOpenHumanContext(normalizedContext);
  const articles = rawArticles
    .filter((article) => !isSourceExcluded(article, excludedSourceKeys))
    .filter((article) => isSourceAllowed(article, allowedAuthors, allowedSourceKeys));
  const wantsAuthorComparison = isAuthorComparisonRequest(`${question}\n${retrievalQuestion}`);
  const ranked = scoreSentences(retrievalQuestion, articles, wantsAuthorComparison ? 18 : 6);
  const topArticles = selectSourceArticles(retrievalQuestion, articles, ranked, 5, {
    diversifyAuthors: wantsAuthorComparison,
  });
  const productInputSummary = buildProductInputSummary({ question, retrievalQuestion, productInput: options.productInput });
  const sourceActionConstraint = buildSourceActionConstraint(retrievalQuestion, ranked);
  const baseProductDiagnosis = buildProductDiagnosis(productInputSummary, retrievalQuestion, {
    hasSourceEvidence: articles.length > 0,
    sourceActionConstraint,
  });
  const baseLearningMemoryReminder = normalizeLearningMemoryReminder(
    options.learningMemoryContext || options.learningMemoryReminder || options.workflowMemory,
  );
  let rankedEvidence = buildRankedEvidence(ranked, topArticles, {
    diversifyAuthors: wantsAuthorComparison,
  });
  if (wantsAuthorComparison) {
    rankedEvidence = ensureAuthorEvidenceCoverage(rankedEvidence, topArticles, retrievalQuestion, 8);
  }
  const sources = topArticles.map((article, index) => ({
    author: article.author,
    date: article.date,
    title: article.title,
    sourceUrl: article.sourceUrl,
    sourcePath: article.sourcePath,
    sourceType: article.sourceType || sourceMaterialKind(article),
    excerpt: sourceExcerptForArticle(article, rankedEvidence.filter((item) => item.sourceIndex === index)),
  }));
  const answerType = detectAnswerType(`${retrievalQuestion}\n${question}`);
  const productDecision = baseProductDiagnosis
    ? buildBusinessDecision(answerType, productInputSummary, {
        status: rankedEvidence.some((item) => isAuthorOriginalSource(topArticles[item.sourceIndex])) ? "source_backed" : "needs_source",
        dataRequests: buildValidationDataRequests(answerType, productInputSummary, sources),
        productDiagnosis: baseProductDiagnosis,
        sourceActionConstraint,
      })
    : undefined;
  const productDiagnosis = baseProductDiagnosis && productDecision
    ? { ...baseProductDiagnosis, decision: productDecision }
    : baseProductDiagnosis;
  const learningMemoryReminder = withLearningMemoryAlignment(baseLearningMemoryReminder, sources, rankedEvidence);
  const sourceTreeCalibration = normalizeSourceTreeCalibration(options.sourceTreeCalibration);
  const answer = buildAnswerText(
    question,
    ranked,
    topArticles,
    retrievalQuestion,
    productInputSummary,
    productDiagnosis,
    learningMemoryReminder,
  );
  const diagnosisPanel = buildDiagnosisPanel(productInputSummary, productDiagnosis, retrievalQuestion);
  const evidenceChain = buildEvidenceChain(question, answer, sources, rankedEvidence, retrievalQuestion);
  const validationPack = buildValidationPack(
    question,
    answer,
    sources,
    retrievalQuestion,
    evidenceChain,
    productInputSummary,
    productDiagnosis,
  );
  const evidenceAudit = buildEvidenceAudit({ question, answer, sources, evidenceChain, rankedEvidence, retrievalQuestion });
  const synthesisAnswer = buildSynthesisAnswer({ question, retrievalQuestion, sources, evidenceChain, evidenceAudit });
  const notebookGuide = buildNotebookGuide({ question, answer, sources, retrievalQuestion, evidenceChain, synthesisAnswer });
  const sourceTrust = buildSourceTrustChain({
    sources,
    evidenceChain,
    evidenceAudit,
    synthesisAnswer,
    productInputSummary,
    sourceTreeCalibration,
  });
  const learningCard = buildLearningCard(question, answer, sources, retrievalQuestion);
  const sourceStudyPack = buildSourceStudyPack({ question, answer, sources, retrievalQuestion, evidenceChain, learningCard });
  const topicSourceTree = buildTopicSourceTree({ question, answer, sources, retrievalQuestion, evidenceChain, learningCard });
  const authorPerspectiveRoom = buildAuthorPerspectiveRoom({
    question,
    retrievalQuestion,
    sources,
    evidenceChain,
    evidenceAudit,
    learningCard,
  });
  const workflowIntent = buildWorkflowIntent({
    question,
    retrievalQuestion,
    sources,
    productInputSummary,
    validationPack,
    learningCard,
    intentPreference: options.intentPreference,
  });
  const learningQueue = buildLearningQueue({
    question,
    retrievalQuestion,
    sources,
    evidenceChain,
    evidenceAudit,
    validationPack,
    learningCard,
    workflowIntent,
  });
  const knowledgeGapRadar = buildKnowledgeGapRadar({
    question,
    retrievalQuestion,
    sources,
    evidenceChain,
    evidenceAudit,
    validationPack,
    sourceTrust,
    sourceStudyPack,
    learningCard,
  });
  const nextBestSource = buildNextBestSourceRoute({
    question,
    retrievalQuestion,
    sources,
    evidenceChain,
    evidenceAudit,
    validationPack,
    sourceStudyPack,
    topicSourceTree,
    knowledgeGapRadar,
  });
  const usageFootprint = buildUsageFootprint({
    question,
    retrievalQuestion,
    answer,
    sources,
    rankedEvidence,
    evidenceChain,
  });

  return {
    question,
    answer,
    sources,
    rankedEvidence,
    sourceScope: buildSourceScopeSummary(rawArticles, articles, allowedAuthors, allowedSourceKeys, { allowedSourceCount }),
    productInputSummary,
    diagnosisPanel,
    validationPack,
    evidenceChain,
    evidenceAudit,
    sourceTrust,
    sourceTreeCalibration,
    synthesisAnswer,
    notebookGuide,
    graph: buildAnswerGraph(question, answer, sources, retrievalQuestion, rankedEvidence, evidenceChain),
    topicSourceTree,
    sourceStudyPack,
    authorPerspectiveRoom,
    learningCard,
    workflowIntent,
    learningQueue,
    knowledgeGapRadar,
    nextBestSource,
    usageFootprint,
    learningMemoryReminder,
    suggestedQuestions: DEFAULT_SUGGESTED_QUESTIONS,
    rawContext: normalizedContext,
  };
}

export function buildUsageFootprint(input = {}) {
  const question = String(input.question || "");
  const retrievalQuestion = String(input.retrievalQuestion || question);
  const answer = String(input.answer || "");
  const sources = Array.isArray(input.sources) ? input.sources : [];
  const rankedEvidence = Array.isArray(input.rankedEvidence) ? input.rankedEvidence : [];
  const evidenceClaims = Array.isArray(input.evidenceChain?.claims) ? input.evidenceChain.claims : [];
  const sourceText = sources
    .slice(0, 5)
    .map((source) => [source?.author, source?.title, source?.excerpt].filter(Boolean).join("\n"))
    .join("\n\n");
  const evidenceText = rankedEvidence
    .slice(0, 8)
    .map((item) => item?.quote || item?.text || "")
    .filter(Boolean)
    .join("\n");
  const claimText = evidenceClaims
    .slice(0, 10)
    .map((claim) => claim?.text || "")
    .filter(Boolean)
    .join("\n");
  const questionTokens = estimateChineseMixedTokens(question);
  const retrievalTokens = estimateChineseMixedTokens(retrievalQuestion);
  const sourceTokens = estimateChineseMixedTokens(`${sourceText}\n${evidenceText}\n${claimText}`);
  const answerTokens = estimateChineseMixedTokens(answer);
  const totalCloudEquivalentTokens = questionTokens + retrievalTokens + sourceTokens + answerTokens;
  return {
    mode: "local_ollama",
    model: "mxbai-embed-large:latest",
    cloudBillableTokens: 0,
    cloudBillableCostText: "当前本机 Ollama 模式不产生 OpenAI 云端 token 费用。",
    estimate: {
      questionTokens,
      retrievalTokens,
      sourceTokens,
      answerTokens,
      totalCloudEquivalentTokens,
    },
    summary: `本轮在本机运行，云端计费 token 为 0；如果改接云模型，这轮大约相当于 ${totalCloudEquivalentTokens} token。`,
    boundary: "这是按问题、检索上下文、来源摘录和答案文本估算的参考值，不是云模型账单；实际费用会随模型、提示词和带入来源长度变化。",
  };
}

function estimateChineseMixedTokens(text) {
  const value = String(text || "").trim();
  if (!value) return 0;
  const cjkCount = (value.match(/[\u3400-\u9fff]/g) || []).length;
  const latinWords = (value.replace(/[\u3400-\u9fff]/g, " ").match(/[a-zA-Z0-9_.:/%-]+/g) || []).length;
  const punctuation = (value.match(/[^\s\w\u3400-\u9fff]/g) || []).length;
  return Math.max(1, Math.ceil(cjkCount * 0.75 + latinWords * 1.25 + punctuation * 0.2));
}

export function buildProductInputSummary(input = {}) {
  const supplied = normalizeProductInput(input.productInput);
  const questionText = String(input.question || "").trim();
  const sourceText = supplied.text || questionText;
  const intake = supplied.intake || buildProductIntake({ text: sourceText });
  const sections = Array.isArray(intake?.sections) ? intake.sections : [];
  const facts = sections
    .filter((section) => section?.id !== "other" && Array.isArray(section?.items) && section.items.length > 0)
    .map((section) => ({
      id: compactGraphLabel(section.id || "item", 40),
      label: compactGraphLabel(section.label || productSectionFallbackLabel(section.id), 40),
      items: uniqueSafeList(section.items, 6, 220),
      missing: uniqueSafeList(section.missing, 4, 80),
    }))
    .filter((section) => section.items.length > 0);
  const missing = uniqueSafeList(intake?.missing, 8, 120);

  if (!supplied.hasValue && facts.length === 0) return undefined;

  return {
    source: "user_input",
    summary:
      facts.length > 0
        ? `本轮识别到 ${facts.length} 类用户产品信息：${facts.map((section) => section.label).join("、")}。`
        : "本轮还没有足够具体的产品信息，回答只能先给检查方向。",
    facts,
    missing,
    caution: "这些是用户提供的诊断输入，不是本地资料证据；系统没有验证图片、后台数据或广告账户。",
  };
}

export function buildEvidenceChain(question, answer, sources = [], rankedEvidence = [], retrievalQuestion = question) {
  const claims = [];
  const safeEvidence = Array.isArray(rankedEvidence)
    ? rankedEvidence.filter((item) => Number.isInteger(item.sourceIndex) && Number(item.score || 0) > 0 && String(item.quote || "").trim())
    : [];
  const questionConcepts = detectGraphConcepts(`${retrievalQuestion}\n${question}`);
  const prioritizedEvidence = [...safeEvidence].sort(
    (a, b) =>
      evidenceConceptMatches(b.quote, questionConcepts) - evidenceConceptMatches(a.quote, questionConcepts) ||
      Number(b.score || 0) - Number(a.score || 0),
  );
  const orderedEvidence = isAuthorComparisonRequest(`${question}\n${retrievalQuestion}`)
    ? diversifyEvidenceByAuthor(prioritizedEvidence, 5)
    : prioritizedEvidence;

  if (sources.length === 0 || orderedEvidence.length === 0) {
    return {
      summary: "这次还没有足够来源支撑。",
      claims: [
        {
          id: "needs-source:0",
          type: "needs_source",
          label: "暂无直接证据",
          canProve: false,
          evidenceKind: "missing_source",
          trustKind: "insufficient",
          trustLabel: "不足以确认",
          trustLevel: "missing",
          canUseAsEvidence: false,
          text: "这次没有从本地资料里找到足够明确的原文证据。",
          validation: "请换一个更具体的问题，或补充产品、关键词、页面、广告数据后再问。",
        },
      ],
    };
  }

  const seenQuotes = new Set();
  orderedEvidence.slice(0, 5).forEach((item, index) => {
    const quoteKey = item.quote.slice(0, 70);
    if (seenQuotes.has(quoteKey)) return;
    seenQuotes.add(quoteKey);
    const source = sources[item.sourceIndex] || {};
    const isAuthorOriginal = isAuthorOriginalSource(source);
    claims.push({
      id: `${isAuthorOriginal ? "source-evidence" : "user-material"}:${Number.isInteger(item.evidenceIndex) ? item.evidenceIndex : index}`,
      type: isAuthorOriginal ? "source_evidence" : "user_material",
      label: isAuthorOriginal ? "资料证据" : "我的资料",
      canProve: isAuthorOriginal,
      evidenceKind: isAuthorOriginal ? "source_quote" : "user_material",
      trustKind: isAuthorOriginal ? "author_original" : "user_material",
      trustLabel: isAuthorOriginal ? "作者原文证据" : "我的资料/用户材料",
      trustLevel: isAuthorOriginal ? "direct" : "user_provided",
      canUseAsEvidence: isAuthorOriginal,
      text: compactGraphLabel(item.quote, 90),
      quote: item.quote,
      sourceIndex: item.sourceIndex,
      author: source.author || item.author || "",
      title: source.title || item.title || "",
      date: source.date || item.date || "",
      basis: isAuthorOriginal
        ? "来自本地作者原文摘录"
        : "来自用户添加的我的资料，只能作为业务材料或诊断输入，不能当作者原文证据",
    });
  });

  if (!claims.some((claim) => claim.type === "source_evidence")) {
    claims.push({
      id: "needs-author-source:0",
      type: "needs_source",
      label: "缺少作者原文",
      canProve: false,
      evidenceKind: "missing_author_source",
      trustKind: "insufficient",
      trustLabel: "缺少作者原文证据",
      trustLevel: "missing",
      canUseAsEvidence: false,
      text: "本轮只找到我的资料或系统整理，尚未绑定三位作者的原文证据。",
      validation: "如要沉淀成作者知识结论，请继续检索张子卿、飞翔的波波或跨境电商长期主义的原文来源。",
    });
  }

  const sections = extractAnswerSections(answer);
  const type = detectAnswerType(`${retrievalQuestion}\n${question}`);
  sections.conclusions.slice(0, 3).forEach((text, index) => {
    claims.push({
      id: `system-inference:${index}`,
      type: "system_inference",
      label: "系统推断",
      canProve: false,
      evidenceKind: "system_inference",
      trustKind: "system_synthesis",
      trustLabel: "二次摘要/系统整理",
      trustLevel: "derived",
      canUseAsEvidence: false,
      text,
      evidenceIndexes: relatedEvidenceIndexes(text, prioritizedEvidence).slice(0, 3),
      basis: "由问题类型、资料证据和亚马逊检查框架综合得到，不等同于原文直接结论",
      validation: buildValidationPrompt(type),
    });
  });

  sections.steps.slice(0, 4).forEach((text, index) => {
    claims.push({
      id: `action-advice:${index}`,
      type: "action_advice",
      label: "行动建议",
      canProve: false,
      evidenceKind: "action_advice",
      trustKind: "system_synthesis",
      trustLabel: "二次摘要/系统整理",
      trustLevel: "derived",
      canUseAsEvidence: false,
      text,
      evidenceIndexes: relatedEvidenceIndexes(text, prioritizedEvidence).slice(0, 3),
      basis: "由资料和亚马逊检查框架转成的执行步骤，不是原文直接结论",
      validation: buildValidationPrompt(type),
    });
  });

  const sourceCount = claims.filter((claim) => claim.type === "source_evidence").length;
  const userMaterialCount = claims.filter((claim) => claim.type === "user_material").length;
  const inferenceCount = claims.filter((claim) => claim.type === "system_inference").length;
  const actionCount = claims.filter((claim) => claim.type === "action_advice").length;
  return {
    summary: `作者原文证据 ${sourceCount} 条，我的资料 ${userMaterialCount} 条，系统推断 ${inferenceCount} 条，行动建议 ${actionCount} 条。`,
    claims,
  };
}

export function buildSourceTrustChain(input = {}) {
  const sources = Array.isArray(input.sources) ? input.sources : [];
  const claims = Array.isArray(input.evidenceChain?.claims) ? input.evidenceChain.claims : [];
  const sourceClaims = claims.filter((claim) => claim?.trustKind === "author_original" && claim.type === "source_evidence");
  const userMaterialClaims = claims.filter((claim) => claim?.trustKind === "user_material" || claim.type === "user_material");
  const derivedClaims = claims.filter((claim) => claim?.trustKind === "system_synthesis");
  const insufficientClaims = claims.filter((claim) => claim?.trustKind === "insufficient" || claim.type === "needs_source");
  const productFactCount = Array.isArray(input.productInputSummary?.facts) ? input.productInputSummary.facts.length : 0;
  const authorSourceIndexes = uniqueOrdered(
    sourceClaims.map((claim) => claim.sourceIndex).filter((value) => Number.isInteger(value) && sources[value]),
    8,
  );
  const status = sourceClaims.length > 0 ? "source_backed" : "needs_source";
  const unsupportedCount = derivedClaims.length + insufficientClaims.length;
  const sourceTree = normalizeSourceTreeCalibration(input.sourceTreeCalibration);
  const summary = sourceClaims.length > 0
    ? `本轮有 ${sourceClaims.length} 条作者原文证据，另有 ${derivedClaims.length} 条系统整理或行动建议需要单独验证。${sourceTree && sourceTree.status !== "empty" ? ` ${sourceTree.summary}` : ""}`
    : "本轮没有可定位的作者原文证据，不能把回答沉淀为可靠结论。";

  return {
    title: "本轮来源核对状态",
    status,
    label: status === "source_backed" ? "有作者原文支撑，但业务适配仍需验证" : "缺少作者原文证据",
    summary,
    boundary: "有来源不等于已经人工核验或业务验证完成；作者原文证据、二次摘要/系统整理、用户产品材料和实验复盘必须分开看。用户产品材料不是作者原文证据，实验复盘不是作者原文证据。",
    sourceTree: sourceTree ? {
      title: sourceTree.title,
      status: sourceTree.status,
      summary: sourceTree.summary,
      boundary: sourceTree.boundary,
      candidateCount: sourceTree.candidateCount,
      resolvedSourceCount: sourceTree.resolvedSourceCount,
      summaryHintCount: sourceTree.summaryHintCount,
    } : undefined,
    categories: [
      {
        id: "author_original",
        label: "作者原文证据",
        status: sourceClaims.length > 0 ? "present" : "missing",
        count: sourceClaims.length,
        description: sourceClaims.length > 0
          ? "可回到本地原文上下文核对，只能说明作者资料里有这段依据。"
          : "当前回答没有可核对的作者原文证据。",
        claimIds: sourceClaims.map((claim) => claim.id).filter(Boolean).slice(0, 8),
        sourceIndexes: authorSourceIndexes,
      },
      {
        id: "system_synthesis",
        label: "二次摘要/系统整理",
        status: derivedClaims.length > 0 ? "present" : "missing",
        count: derivedClaims.length,
        description: "用于把原文和问题整理成要点或步骤，不能单独当作者原文证据。",
        claimIds: derivedClaims.map((claim) => claim.id).filter(Boolean).slice(0, 8),
      },
      {
        id: "product_material",
        label: "用户产品材料",
        status: productFactCount > 0 || userMaterialClaims.length > 0 ? "present" : "missing",
        count: productFactCount + userMaterialClaims.length,
        description: productFactCount > 0 || userMaterialClaims.length > 0
          ? "只用于判断你的产品场景，不会进入作者原文证据，也不能作为已采纳作者来源。"
          : "本轮未带入具体产品材料；业务判断只能停留在检查方向。",
        claimIds: userMaterialClaims.map((claim) => claim.id).filter(Boolean).slice(0, 8),
      },
      {
        id: "experiment_review",
        label: "实验/复盘",
        status: "missing",
        count: 0,
        description: "本轮问答未回填实验结果；实验复盘不是作者原文证据，只能说明你的历史验证结果。",
      },
      {
        id: "insufficient",
        label: "不足以确认",
        status: insufficientClaims.length > 0 || unsupportedCount > 0 ? "present" : "clear",
        count: insufficientClaims.length,
        description: insufficientClaims.length > 0
          ? "缺少可核对来源时，不允许显示为已确认结论。"
          : "仍需用原文上下文和真实业务数据复核，不能只凭系统整理下最终结论。",
        claimIds: insufficientClaims.map((claim) => claim.id).filter(Boolean).slice(0, 8),
      },
    ],
  };
}

export function buildEvidenceAudit(input = {}) {
  const sources = Array.isArray(input.sources) ? input.sources : [];
  const claims = Array.isArray(input.evidenceChain?.claims) ? input.evidenceChain.claims : [];
  const sourceEvidence = claims.filter((claim) => claim.type === "source_evidence");
  const needsSource = claims.filter((claim) => claim.type === "needs_source");
  const systemInferences = claims.filter((claim) => claim.type === "system_inference");
  const actionAdvice = claims.filter((claim) => claim.type === "action_advice");
  const sourceIndexes = uniqueOrdered(sourceEvidence.map((claim) => claim.sourceIndex).filter((value) => Number.isInteger(value)), 8);
  const conflictSignals = detectConflictSignals(sourceEvidence, sources);
  const supportingConflictReasonCount = conflictSignals.reduce((sum, item) => sum + (Array.isArray(item.supportingReasons) ? item.supportingReasons.length : 0), 0);
  const datedSources = sources
    .map((source) => ({ source, parsed: parseSourceDate(source.date) }))
    .filter((item) => item.parsed);
  const newest = datedSources.length
    ? datedSources.slice().sort((a, b) => b.parsed.getTime() - a.parsed.getTime())[0]
    : null;
  const oldest = datedSources.length
    ? datedSources.slice().sort((a, b) => a.parsed.getTime() - b.parsed.getTime())[0]
    : null;
  const newestAgeDays = newest ? Math.floor((Date.now() - newest.parsed.getTime()) / 86400000) : null;
  const dateSpreadDays = newest && oldest ? Math.floor((newest.parsed.getTime() - oldest.parsed.getTime()) / 86400000) : 0;
  const sourceStatus = sourceEvidence.length >= 3 && sourceIndexes.length >= 2
    ? "ok"
    : sourceEvidence.length > 0
      ? "warn"
      : "missing";
  const inferenceStatus = needsSource.length > 0
    ? "missing"
    : (systemInferences.length > sourceEvidence.length ? "warn" : "ok");
  const freshnessStatus = newestAgeDays === null
    ? "warn"
    : newestAgeDays > 730 || dateSpreadDays > 730
      ? "warn"
      : "ok";
  const unsupportedCount = needsSource.length + systemInferences.length + actionAdvice.length;
  const conflictStatus = conflictSignals.length > 0 ? "missing" : "ok";
  const warnCount = [sourceStatus, inferenceStatus, freshnessStatus, conflictStatus].filter((status) => status === "warn").length;
  const level = sourceStatus === "missing" || needsSource.length > 0 || conflictSignals.length > 0 || (sourceStatus === "warn" && warnCount >= 3)
    ? "low"
    : sourceStatus === "ok" && inferenceStatus === "ok" && freshnessStatus === "ok" && conflictStatus === "ok"
      ? "high"
      : "medium";
  const checks = [
    {
      id: "source_coverage",
      label: "来源覆盖",
      status: sourceStatus,
      message: sourceEvidence.length > 0
        ? `有 ${sourceEvidence.length} 条原文证据，覆盖 ${sourceIndexes.length || 1} 个来源。`
        : "这次没有找到可直接定位的原文证据。",
      sourceIndexes,
    },
    {
      id: "claim_boundary",
      label: "结论边界",
      status: inferenceStatus,
      message: unsupportedCount > 0
        ? `${systemInferences.length} 条是系统推断，${actionAdvice.length} 条是行动建议，需要结合你的产品数据验证。`
        : "当前回答主要由原文证据构成。",
    },
    {
      id: "freshness",
      label: "资料新旧",
      status: freshnessStatus,
      message: newest
        ? `最新来源日期：${newest.source.date || "未知"}；较旧资料需要结合当前平台规则复核。`
        : "来源没有稳定日期，时间有效性需要人工复核。",
    },
    {
      id: "conflict_scan",
      label: "冲突风险提示",
      status: conflictStatus,
      message: conflictSignals.length > 0
        ? `发现 ${conflictSignals.length} 个可能冲突的概念：${conflictSignals.map(conflictSignalLabel).join("、")}。${supportingConflictReasonCount > 0 ? `另有 ${supportingConflictReasonCount} 个辅助判断理由。` : ""}需要打开来源对比后再采纳结论。`
        : "未发现明显冲突，但这只是轻量扫描，不能证明资料之间没有冲突；若不同作者观点相反，需要专门追问对比。",
      sourceIndexes: uniqueOrdered(conflictSignals.flatMap((item) => item.sourceIndexes || []), 8),
    },
  ];
  return {
    level,
    label: evidenceAuditLabel(level),
    summary: evidenceAuditSummary(level, sourceEvidence.length, unsupportedCount, conflictSignals.length),
    counts: {
      sources: sources.length,
      sourceEvidence: sourceEvidence.length,
      systemInferences: systemInferences.length,
      actionAdvice: actionAdvice.length,
      needsSource: needsSource.length,
      conflictSignals: conflictSignals.length,
      supportingConflictReasons: supportingConflictReasonCount,
    },
    checks,
    conflictSignals,
    caution: "引用核对用于帮助你判断答案边界，不会替代原文阅读、人工核验和真实业务验证。",
  };
}

function normalizeSourceTreeCalibration(calibration) {
  if (!calibration || typeof calibration !== "object") return undefined;
  const safeText = (value, maxLength = 220) => String(value || "").trim().slice(0, maxLength);
  const status = ["active", "unresolved", "summary_only", "empty"].includes(calibration.status)
    ? calibration.status
    : "empty";
  const candidates = Array.isArray(calibration.candidates)
    ? calibration.candidates
        .map((item, index) => ({
          id: safeText(item?.id || `source-tree:candidate:${index}`, 80),
          type: safeText(item?.type || "route_hint", 40),
          label: safeText(item?.label, 120),
          owner: safeText(item?.owner, 80),
          sourceId: safeText(item?.sourceId, 260),
          sourceRef: safeText(item?.sourceRef, 320),
          chunkCount: Number.isFinite(Number(item?.chunkCount)) ? Math.max(0, Number(item.chunkCount)) : 0,
          matchedOriginalSource: item?.matchedOriginalSource === true,
          matchedTitle: safeText(item?.matchedTitle, 160),
          canUseAsEvidence: false,
        }))
        .filter((item) => item.label || item.sourceId || item.sourceRef)
        .slice(0, 8)
    : [];
  const summaries = Array.isArray(calibration.summaries)
    ? calibration.summaries
        .map((item, index) => ({
          id: safeText(item?.id || `source-tree:summary:${index}`, 80),
          type: "summary_hint",
          label: safeText(item?.label, 120),
          treeId: safeText(item?.treeId, 120),
          excerpt: safeText(item?.excerpt, 240),
          canUseAsEvidence: false,
        }))
        .filter((item) => item.label || item.excerpt)
        .slice(0, 4)
    : [];
  return {
    title: safeText(calibration.title || "OpenHuman 来源树辅助检索", 100),
    status,
    terms: Array.isArray(calibration.terms) ? calibration.terms.map((term) => safeText(term, 40)).filter(Boolean).slice(0, 12) : [],
    candidateCount: Number.isFinite(Number(calibration.candidateCount)) ? Math.max(0, Number(calibration.candidateCount)) : candidates.length,
    resolvedSourceCount: Number.isFinite(Number(calibration.resolvedSourceCount))
      ? Math.max(0, Number(calibration.resolvedSourceCount))
      : candidates.filter((item) => item.matchedOriginalSource).length,
    summaryHintCount: Number.isFinite(Number(calibration.summaryHintCount)) ? Math.max(0, Number(calibration.summaryHintCount)) : summaries.length,
    summary: safeText(calibration.summary, 320),
    boundary: safeText(
      calibration.boundary || "来源树摘要和候选片段只负责帮系统找路，不能当作者原文证据；回答里的引用必须回到本地作者原文上下文后才可采纳。",
      420,
    ),
    candidates,
    summaries,
  };
}

export function buildSynthesisAnswer(input = {}) {
  const question = String(input.question || "").trim();
  const retrievalQuestion = String(input.retrievalQuestion || question).trim();
  const sources = Array.isArray(input.sources) ? input.sources : [];
  const claims = Array.isArray(input.evidenceChain?.claims) ? input.evidenceChain.claims : [];
  const sourceClaims = claims
    .filter((claim) => claim?.type === "source_evidence" && claim.id && Number.isInteger(claim.sourceIndex))
    .filter((claim) => sources[claim.sourceIndex]);
  const sourceIndexes = uniqueOrdered(sourceClaims.map((claim) => claim.sourceIndex).filter(Number.isInteger), 8);
  const authors = uniqueOrdered(
    sourceClaims
      .map((claim) => claim.author || sources[claim.sourceIndex]?.author || "")
      .filter(Boolean),
    8,
  );
  const sourceCoverage = {
    sourceCount: sourceIndexes.length,
    evidenceCount: sourceClaims.length,
    authorCount: authors.length,
    authors,
  };
  const sourceClaimIds = uniqueOrdered(sourceClaims.map((claim) => claim.id).filter(Boolean), 12);

  if (sourceClaims.length === 0) {
    return {
      title: "本轮综合答案",
      status: "needs_source",
      summary: "这轮还没有可绑定的作者原文证据，不能生成正式综合结论。",
      sourceCoverage,
      sourceClaimIds: [],
      points: [],
      authorPerspectives: [],
      conflicts: [],
      gaps: [
        {
          id: "synthesis-gap:source",
          label: "缺少作者原文来源",
          reason: "需要先找到可定位的作者原文证据，再把回答整理成可采纳的综合结论。",
        },
      ],
      boundary: "当前没有作者原文证据，不能生成正式综合结论；用户数据、历史档案和系统推断都不能替代来源。",
    };
  }

  const conceptText = [
    ...sourceClaims.map((claim) => `${claim.text || ""}\n${claim.quote || ""}\n${claim.title || ""}`),
  ].join("\n");
  const concepts = detectGraphConcepts(conceptText).slice(0, 6);
  const requestedConcepts = detectGraphConcepts(`${retrievalQuestion}\n${question}`).slice(0, 8);
  const unsupportedConcepts = requestedConcepts.filter((concept) => !concepts.includes(concept));
  const conflicts = buildSynthesisConflicts(input.evidenceAudit?.conflictSignals, sources);
  const status = conflicts.length > 0 ? "needs_review" : "source_backed";
  const points = buildSynthesisPoints(sourceClaims, sources, concepts);
  const gaps = buildSynthesisGaps({ sourceCoverage, conflicts, unsupportedConcepts, retrievalQuestion, question });

  return {
    title: "本轮综合答案",
    status,
    summary: buildSynthesisSummary({ concepts, sourceCoverage, conflicts }),
    sourceCoverage,
    sourceClaimIds,
    points,
    authorPerspectives: buildSynthesisAuthorPerspectives(sourceClaims, sources),
    conflicts,
    gaps,
    boundary: "这是系统综合，不是新的作者原文证据；只有下方绑定的作者原文片段可作为来源支撑，是否适用于你的产品还要用业务数据验证。",
  };
}

function buildSynthesisPoints(sourceClaims, sources, concepts) {
  const candidates = concepts.length > 0
    ? concepts.map((concept) => {
        const related = sourceClaims.filter((claim) => textMentionsConcept(`${claim.text || ""}\n${claim.quote || ""}\n${claim.title || ""}`, concept));
        return related.length > 0 ? { concept, claims: related } : null;
      }).filter(Boolean)
    : sourceClaims.map((claim, index) => ({ concept: `要点 ${index + 1}`, claims: [claim] }));
  const seenConcepts = new Set();
  const points = [];

  candidates.forEach((candidate, index) => {
    if (!candidate.concept || seenConcepts.has(candidate.concept)) return;
    seenConcepts.add(candidate.concept);
    const relatedClaims = candidate.claims.slice(0, 3);
    const support = relatedClaims.map((claim) => synthesisSupportFromClaim(claim, sources[claim.sourceIndex]));
    const sourceIndexes = uniqueOrdered(support.map((item) => item.sourceIndex).filter(Number.isInteger), 5);
    const claimIds = uniqueOrdered(support.map((item) => item.claimId).filter(Boolean), 5);
    if (support.length === 0) return;
    points.push({
      id: `synthesis-point:${index}`,
      label: compactGraphLabel(`${candidate.concept}：${support[0].quote}`, 90),
      text: buildSynthesisPointText(candidate.concept, support),
      identity: "系统综合",
      evidenceKind: "system_synthesis",
      canUseAsEvidence: false,
      isInference: true,
      confidence: support.length >= 2 && sourceIndexes.length >= 2 ? "medium" : "low",
      basis: "由本轮已定位的作者原文证据综合得到；表达本身不是作者原话。",
      claimIds,
      sourceIndexes,
      support,
    });
  });

  return points.slice(0, 5);
}

function buildSynthesisPointText(concept, support) {
  const authorCount = uniqueOrdered(support.map((item) => item.author).filter(Boolean), 4).length;
  const sourceCount = uniqueOrdered(support.map((item) => item.sourceIndex).filter(Number.isInteger), 4).length;
  if (concept === "主图" || concept === "点击率") {
    return `围绕「${concept}」，本轮来源提示先把搜索结果里的视觉点击入口单独检查，再看页面承接。`;
  }
  if (concept === "转化率" || concept === "Listing") {
    return `围绕「${concept}」，本轮来源提示不要只看单一指标，要结合页面内容、产品力和成交条件复核。`;
  }
  if (concept === "广告") {
    return "围绕「广告」，本轮来源提示广告更像放大器，需要先确认页面和流量是否匹配。";
  }
  return `围绕「${concept}」，本轮综合了 ${sourceCount} 个来源、${authorCount || 1} 位作者的原文片段，适用性还要回到你的产品数据验证。`;
}

function synthesisSupportFromClaim(claim, source = {}) {
  return {
    claimId: claim.id,
    sourceIndex: claim.sourceIndex,
    identity: "作者原文",
    evidenceKind: "source_evidence",
    author: claim.author || source.author || "",
    title: claim.title || source.title || "",
    date: claim.date || source.date || "",
    quote: compactGraphLabel(claim.quote || claim.text || source.excerpt || "", 260),
  };
}

function buildSynthesisAuthorPerspectives(sourceClaims, sources) {
  const groups = new Map();
  sourceClaims.forEach((claim) => {
    const source = sources[claim.sourceIndex] || {};
    const author = claim.author || source.author || "";
    if (!author) return;
    if (!groups.has(author)) groups.set(author, []);
    groups.get(author).push(claim);
  });
  return [...groups.entries()].slice(0, 4).map(([author, claims], index) => ({
    id: `synthesis-author:${index}`,
    author,
    claimIds: claims.map((claim) => claim.id).filter(Boolean).slice(0, 5),
    sourceIndexes: uniqueOrdered(claims.map((claim) => claim.sourceIndex).filter(Number.isInteger), 5),
    summary: compactGraphLabel(claims.map((claim) => claim.quote || claim.text || "").filter(Boolean).join(" / "), 220),
  }));
}

function buildSynthesisConflicts(conflictSignals, sources) {
  if (!Array.isArray(conflictSignals)) return [];
  return conflictSignals.slice(0, 4).map((signal, index) => ({
    id: `synthesis-conflict:${index}`,
    concept: compactGraphLabel(conflictConceptLabel(signal.concept, signal.relatedConcepts), 80),
    message: compactGraphLabel(signal.comparison?.summary || signal.comparison?.suggestedCheck || "来源中存在可能相反的观点，需要人工对比后再采纳。", 240),
    sourceIndexes: Array.isArray(signal.sourceIndexes)
      ? signal.sourceIndexes.filter((value) => Number.isInteger(value) && sources[value]).slice(0, 6)
      : [],
  })).filter((item) => item.concept || item.message);
}

function buildSynthesisGaps(input = {}) {
  const gaps = [];
  const sourceCoverage = input.sourceCoverage || {};
  const unsupportedConcepts = Array.isArray(input.unsupportedConcepts) ? input.unsupportedConcepts.filter(Boolean).slice(0, 4) : [];
  unsupportedConcepts.forEach((concept) => {
    gaps.push({
      id: `synthesis-gap:concept:${safeGraphId(concept)}`,
      label: `${concept}缺少直接来源`,
      reason: `你的问题提到了「${concept}」，但本轮可引用原文没有直接支撑这个概念，不能把其他来源硬绑定成${concept}结论。`,
    });
  });
  if (Number(sourceCoverage.sourceCount || 0) < 2) {
    gaps.push({
      id: "synthesis-gap:independent-source",
      label: "独立来源不足",
      reason: "当前综合主要依赖少数来源，不能当作多方验证后的结论。",
    });
  }
  if (Number(sourceCoverage.authorCount || 0) < 2) {
    gaps.push({
      id: "synthesis-gap:author",
      label: "作者视角不足",
      reason: "当前综合没有覆盖多个作者的差异观点，后续可追问作者对比。",
    });
  }
  if (Array.isArray(input.conflicts) && input.conflicts.length > 0) {
    gaps.push({
      id: "synthesis-gap:conflict",
      label: "存在观点冲突",
      reason: "需要先打开冲突来源逐条核对，再决定哪一侧更适合你的产品。",
    });
  }
  return gaps.slice(0, 5);
}

function buildSynthesisSummary(input = {}) {
  const concepts = Array.isArray(input.concepts) ? input.concepts.slice(0, 4) : [];
  const coverage = input.sourceCoverage || {};
  const conceptLabel = concepts.length > 0 ? concepts.join("、") : "本轮问题";
  if (Array.isArray(input.conflicts) && input.conflicts.length > 0) {
    return `本轮围绕 ${conceptLabel} 做了来源综合，但发现可能相反的观点，需要先对比来源后再采纳。`;
  }
  return `本轮围绕 ${conceptLabel} 综合了 ${Number(coverage.evidenceCount || 0)} 条作者原文证据，覆盖 ${Number(coverage.sourceCount || 0)} 个来源。`;
}

export function buildNotebookGuide(input = {}) {
  const question = String(input.question || "").trim();
  const retrievalQuestion = String(input.retrievalQuestion || question).trim();
  const answer = String(input.answer || "");
  const sources = Array.isArray(input.sources) ? input.sources : [];
  const claims = Array.isArray(input.evidenceChain?.claims) ? input.evidenceChain.claims : [];
  const sourceClaims = claims
    .filter((claim) => claim?.type === "source_evidence" && claim.id && Number.isInteger(claim.sourceIndex))
    .filter((claim) => sources[claim.sourceIndex])
    .slice(0, 6);
  if (sourceClaims.length === 0) {
    return {
      title: "本轮学习简报",
      status: "needs_source",
      summary: "当前没有可绑定的作者原文证据，不能生成正式学习简报。",
      sourceCoverage: { sourceCount: 0, evidenceCount: 0, authorCount: 0, authors: [] },
      briefing: [],
      faq: [],
      quiz: [],
      glossary: [],
      gaps: [
        {
          id: "notebook-gap:source",
          label: "缺少作者原文来源",
          reason: "没有可定位的作者原文证据时，只能继续检索，不能把回答包装成学习讲义。",
          identity: "系统整理",
          evidenceKind: "notebook_gap",
          canUseAsEvidence: false,
          claimIds: [],
          sourceIndexes: [],
          prompt: `请重新检索「${compactGraphLabel(retrievalQuestion || question || "这个问题", 80)}」的作者原文证据。`,
        },
      ],
      nextPrompts: [`请重新检索「${compactGraphLabel(retrievalQuestion || question || "这个问题", 80)}」的作者原文证据。`],
      boundary: "当前没有作者原文证据，不能生成正式学习简报；系统整理、历史档案和用户业务材料都不能替代来源。",
    };
  }

  const sourceIndexes = uniqueOrdered(sourceClaims.map((claim) => claim.sourceIndex).filter(Number.isInteger), 8);
  const authors = uniqueOrdered(sourceClaims.map((claim) => claim.author || sources[claim.sourceIndex]?.author || "").filter(Boolean), 8);
  const conceptText = [
    retrievalQuestion,
    question,
    answer,
    ...sourceClaims.map((claim) => `${claim.text || ""}\n${claim.quote || ""}\n${claim.title || ""}`),
  ].join("\n");
  const concepts = detectGraphConcepts(conceptText).slice(0, 8);
  const sourceCoverage = {
    sourceCount: sourceIndexes.length,
    evidenceCount: sourceClaims.length,
    authorCount: authors.length,
    authors,
  };
  return {
    title: "本轮学习简报",
    status: "source_backed",
    summary: `基于 ${sourceCoverage.evidenceCount} 条作者原文证据整理，覆盖 ${sourceCoverage.sourceCount} 个来源、${sourceCoverage.authorCount || 1} 位作者。`,
    sourceCoverage,
    briefing: buildNotebookBriefing(sourceClaims, sources, concepts),
    faq: buildNotebookFaq(sourceClaims, sources, concepts, retrievalQuestion || question),
    quiz: buildNotebookQuiz(sourceClaims, sources),
    glossary: buildNotebookGlossary(concepts, sourceClaims, sources),
    gaps: buildNotebookGaps({ sourceCoverage }),
    nextPrompts: [
      "请把本轮学习简报转成 7 天复习计划，并标明每天要回看的来源。",
      "请基于这份简报生成一张 Listing/主图检查清单。",
      "请指出这份简报里哪些结论还需要我的产品数据验证。",
    ],
    boundary: "本轮学习简报是系统整理，不是作者原文证据；每个要点必须回到绑定来源核对，用户产品材料和实验复盘不能替代作者原文。",
  };
}

function buildNotebookBriefing(sourceClaims, sources, concepts) {
  return sourceClaims.slice(0, 4).map((claim, index) => {
    const source = sources[claim.sourceIndex] || {};
    const concept = concepts.find((label) => textMentionsConcept(`${claim.text || ""}\n${claim.quote || ""}\n${claim.title || ""}`, label)) || `要点 ${index + 1}`;
    const quote = compactGraphLabel(claim.quote || claim.text || source.excerpt || "", 260);
    return {
      id: `notebook-brief:${index}`,
      label: compactGraphLabel(concept, 90),
      text: notebookBriefingText(concept, quote),
      identity: "系统整理",
      evidenceKind: "notebook_brief",
      canUseAsEvidence: false,
      claimIds: [claim.id].filter(Boolean),
      sourceIndexes: [claim.sourceIndex].filter(Number.isInteger),
      quote,
      prompt: `请展开解释这个学习要点，并只引用绑定的作者原文：${quote}`,
    };
  });
}

function notebookBriefingText(concept, quote) {
  if (concept === "主图" || concept === "点击率") return `先把「${concept}」当成搜索入口问题看：它负责让用户愿不愿意点进来，再和页面承接分开判断。`;
  if (concept === "转化率" || concept === "Listing") return `围绕「${concept}」，不要只看一个数字，要回到页面解释力、产品力和成交条件一起复核。`;
  if (concept === "广告") return "广告更适合用来放大已成立的页面和产品力，不能替代 Listing 基本面。";
  return `围绕「${concept}」，本轮来源给出的学习重点是：${compactGraphLabel(quote, 120)}`;
}

function buildNotebookFaq(sourceClaims, sources, concepts, question) {
  const primaryConcepts = concepts.length ? concepts.slice(0, 3) : ["本轮问题"];
  return primaryConcepts.map((concept, index) => {
    const relatedClaims = sourceClaims.filter((claim) => textMentionsConcept(`${claim.text || ""}\n${claim.quote || ""}\n${claim.title || ""}`, concept));
    const supportClaims = (relatedClaims.length ? relatedClaims : sourceClaims).slice(0, 2);
    return {
      id: `notebook-faq:${index}`,
      question: index === 0 ? `这轮问题「${compactGraphLabel(question, 60)}」最先应该理解什么？` : `围绕「${concept}」我应该记住哪条判断？`,
      answer: supportClaims
        .map((claim) => compactGraphLabel(claim.quote || claim.text || sources[claim.sourceIndex]?.excerpt || "", 110))
        .filter(Boolean)
        .join("；") || `先回到本轮绑定来源，核对「${concept}」是否真的被原文支持。`,
      identity: "系统整理",
      evidenceKind: "notebook_faq",
      canUseAsEvidence: false,
      claimIds: supportClaims.map((claim) => claim.id).filter(Boolean).slice(0, 4),
      sourceIndexes: uniqueOrdered(supportClaims.map((claim) => claim.sourceIndex).filter(Number.isInteger), 4),
      prompt: `请继续围绕「${concept}」追问，并只使用本轮已绑定来源。`,
    };
  });
}

function buildNotebookQuiz(sourceClaims, sources) {
  return sourceClaims.slice(0, 3).map((claim, index) => {
    const source = sources[claim.sourceIndex] || {};
    const quote = compactGraphLabel(claim.quote || claim.text || source.excerpt || "", 220);
    return {
      id: `notebook-quiz:${index}`,
      question: index === 0 ? "这条来源提醒我先拆哪一个问题？" : "这条来源能支持什么学习判断？",
      answer: quote,
      identity: "系统整理",
      evidenceKind: "notebook_quiz",
      canUseAsEvidence: false,
      claimIds: [claim.id].filter(Boolean),
      sourceIndexes: [claim.sourceIndex].filter(Number.isInteger),
      prompt: `请把这道复习题改成可执行检查动作：${quote}`,
    };
  });
}

function buildNotebookGlossary(concepts, sourceClaims, sources) {
  return concepts.slice(0, 6).map((concept, index) => {
    const relatedClaims = sourceClaims.filter((claim) => textMentionsConcept(`${claim.text || ""}\n${claim.quote || ""}\n${claim.title || ""}`, concept));
    const supportClaims = (relatedClaims.length ? relatedClaims : sourceClaims.slice(index, index + 1)).slice(0, 2);
    return {
      id: `notebook-glossary:${safeGraphId(concept || index)}`,
      term: concept,
      definition: notebookGlossaryDefinition(concept),
      identity: "系统整理",
      canUseAsEvidence: false,
      claimIds: supportClaims.map((claim) => claim.id).filter(Boolean).slice(0, 3),
      sourceIndexes: uniqueOrdered(supportClaims.map((claim) => claim.sourceIndex).filter((sourceIndex) => Number.isInteger(sourceIndex) && sources[sourceIndex]), 3),
    };
  });
}

function notebookGlossaryDefinition(concept) {
  if (concept === "主图") return "搜索结果里最先影响用户是否点击的视觉入口。";
  if (concept === "点击率") return "入口吸引力指标，需要和页面转化分开诊断。";
  if (concept === "转化率") return "页面、产品力、价格、评价和流量匹配共同影响的成交指标。";
  if (concept === "Listing") return "承接搜索意图并完成说服的商品页面系统。";
  if (concept === "广告") return "放大流量和验证词的工具，不应替代产品与页面基本面。";
  return `本轮来源中反复出现的学习概念：${concept}`;
}

function buildNotebookGaps(input = {}) {
  const coverage = input.sourceCoverage || {};
  const gaps = [];
  if (Number(coverage.sourceCount || 0) < 2) {
    gaps.push({
      id: "notebook-gap:source-count",
      label: "独立来源不足",
      reason: "讲义主要依赖少数来源，适合作为学习线索，不适合直接当成多方验证结论。",
      identity: "系统整理",
      evidenceKind: "notebook_gap",
      canUseAsEvidence: false,
      claimIds: [],
      sourceIndexes: [],
    });
  }
  if (Number(coverage.authorCount || 0) < 2) {
    gaps.push({
      id: "notebook-gap:author-count",
      label: "作者视角不足",
      reason: "当前作者覆盖较少，遇到重大决策时还需要继续查其他作者或反例。",
      identity: "系统整理",
      evidenceKind: "notebook_gap",
      canUseAsEvidence: false,
      claimIds: [],
      sourceIndexes: [],
    });
  }
  gaps.push({
    id: "notebook-gap:business-data",
    label: "缺少你的产品数据",
    reason: "学习简报不能替代业务判断；主图、Listing、广告和选品问题仍需要你的点击率、转化率、竞品、评价和利润数据验证。",
    identity: "系统整理",
    evidenceKind: "notebook_gap",
    canUseAsEvidence: false,
    claimIds: [],
    sourceIndexes: [],
    prompt: "我补充产品数据后，请把这份学习简报转成具体判断。",
  });
  return gaps.slice(0, 4);
}

function detectConflictSignals(sourceEvidence = [], sources = []) {
  const buckets = new Map();
  sourceEvidence.forEach((claim) => {
    const text = `${claim.text || ""}\n${claim.quote || ""}`;
    conflictConceptsForText(text).forEach((concept) => {
      const stance = conflictStanceForText(text, concept);
      if (!stance) return;
      if (!buckets.has(concept)) buckets.set(concept, { support: [], caution: [] });
      const source = Number.isInteger(claim.sourceIndex) ? sources[claim.sourceIndex] || {} : {};
      buckets.get(concept)[stance].push({
        sourceIndex: claim.sourceIndex,
        title: claim.title || source.title || "",
        author: claim.author || source.author || "",
        date: claim.date || source.date || "",
        sourceUrl: source.sourceUrl || "",
        sourcePath: source.sourcePath || "",
        quote: compactGraphLabel(claim.quote || claim.text || "", 110),
      });
    });
  });

  const signals = [...buckets.entries()]
    .map(([concept, bucket]) => {
      const supportIndexes = uniqueOrdered(bucket.support.map((item) => item.sourceIndex).filter((value) => Number.isInteger(value)), 8);
      const cautionIndexes = uniqueOrdered(bucket.caution.map((item) => item.sourceIndex).filter((value) => Number.isInteger(value)), 8);
      const hasDifferentSource = supportIndexes.some((index) => !cautionIndexes.includes(index))
        || cautionIndexes.some((index) => !supportIndexes.includes(index));
      if (bucket.support.length === 0 || bucket.caution.length === 0 || !hasDifferentSource) return null;
      const support = bucket.support.slice(0, 2);
      const caution = bucket.caution.slice(0, 2);
      return {
        concept,
        sourceIndexes: uniqueOrdered([...supportIndexes, ...cautionIndexes], 8),
        support,
        caution,
        comparison: buildConflictComparison(concept, conflictRepresentative(support), conflictRepresentative(caution)),
      };
    })
    .filter(Boolean);
  return demoteSupportingConflictReasons(mergeConflictSignals(signals)).slice(0, 4);
}

function mergeConflictSignals(signals = []) {
  const merged = [];
  const groups = new Map();
  signals.forEach((signal) => {
    const family = conflictFamilyForConcept(signal.concept);
    const key = `${family}:${conflictSourceSignature(signal.sourceIndexes)}`;
    const existing = groups.get(key);
    if (!existing) {
      const copy = { ...signal, relatedConcepts: [] };
      groups.set(key, copy);
      merged.push(copy);
      return;
    }
    const concepts = uniqueOrdered([existing.concept, ...(existing.relatedConcepts || []), signal.concept, ...(signal.relatedConcepts || [])], 8);
    existing.concept = preferredConflictConcept(concepts, family);
    existing.relatedConcepts = concepts.filter((concept) => concept !== existing.concept);
    existing.sourceIndexes = uniqueOrdered([...(existing.sourceIndexes || []), ...(signal.sourceIndexes || [])], 8);
    existing.support = mergeConflictItems(existing.support, signal.support, 2);
    existing.caution = mergeConflictItems(existing.caution, signal.caution, 2);
    existing.comparison = buildConflictComparison(
      existing.concept,
      conflictRepresentative(existing.support),
      conflictRepresentative(existing.caution),
      existing.relatedConcepts,
    );
  });
  return merged;
}

function demoteSupportingConflictReasons(signals = []) {
  const result = [];
  const demoted = new Set();
  signals.forEach((signal, index) => {
    if (demoted.has(index)) return;
    const supportingReasons = [];
    signals.forEach((candidate, candidateIndex) => {
      if (candidateIndex === index || demoted.has(candidateIndex)) return;
      if (!shouldDemoteConflictSignal(signal, candidate)) return;
      supportingReasons.push(buildSupportingConflictReason(candidate));
      demoted.add(candidateIndex);
    });
    const relatedConcepts = mergeConflictRelatedConcepts(signal, supportingReasons);
    const supporting = mergeSupportingReasons(signal.supportingReasons, supportingReasons);
    result.push({
      ...signal,
      role: signal.role || "primary",
      relatedConcepts,
      supportingReasons: supporting,
      comparison: buildConflictComparison(
        signal.concept,
        conflictRepresentative(signal.support),
        conflictRepresentative(signal.caution),
        relatedConcepts,
        supporting,
      ),
    });
  });
  return result;
}

function mergeConflictRelatedConcepts(signal = {}, supportingReasons = []) {
  let concepts = [signal.concept, ...(signal.relatedConcepts || [])];
  if (signal.concept === "主图" && conflictSignalText(signal).includes("点击率")) {
    concepts.push("点击率");
  }
  supportingReasons.forEach((reason) => {
    if (reason?.concept === "转化率" && signal.concept === "主图") concepts.push("点击率");
  });
  concepts = uniqueOrdered(concepts.filter(Boolean), 8);
  return concepts.filter((concept) => concept !== signal.concept);
}

function shouldDemoteConflictSignal(primary, candidate) {
  if (conflictFamilyForConcept(primary?.concept) !== "visual_click") return false;
  if (candidate?.concept !== "转化率") return false;
  const sharedSources = sourceOverlapCount(primary.sourceIndexes, candidate.sourceIndexes);
  if (sharedSources < 2) return false;
  const candidateText = conflictSignalText(candidate);
  if (/Listing|页面承接|副图|五点|A\+|A＋|页面/.test(candidateText) && /必须先|必须优先|核心瓶颈|先.*(改|优化).*Listing|先.*(改|优化).*页面/.test(candidateText)) {
    return false;
  }
  const visualText = conflictSignalText(primary);
  return /主图|首图|点击率|CTR|视觉/.test(visualText) && /评价|价格|页面承接|流量质量|转化率|CVR/.test(candidateText);
}

function sourceOverlapCount(left = [], right = []) {
  const rightSet = new Set((Array.isArray(right) ? right : []).filter((value) => Number.isInteger(value)));
  return (Array.isArray(left) ? left : []).filter((value) => rightSet.has(value)).length;
}

function conflictSignalText(signal = {}) {
  return [
    signal.concept,
    ...(signal.relatedConcepts || []),
    ...(signal.support || []).map((item) => item.quote || item.title || ""),
    ...(signal.caution || []).map((item) => item.quote || item.title || ""),
  ].join("\n");
}

function buildSupportingConflictReason(signal = {}) {
  const label = conflictSignalLabel(signal);
  return {
    concept: signal.concept,
    relatedConcepts: Array.isArray(signal.relatedConcepts) ? signal.relatedConcepts : [],
    sourceIndexes: Array.isArray(signal.sourceIndexes) ? signal.sourceIndexes.filter((value) => Number.isInteger(value)).slice(0, 8) : [],
    summary: `${label}更适合作为辅助验证因素：先用它判断主冲突是否真的成立，再决定是否单独展开页面转化问题。`,
    suggestedCheck: conflictSuggestedCheck(signal.concept),
  };
}

function mergeSupportingReasons(left = [], right = []) {
  const seen = new Set();
  const merged = [];
  [...(Array.isArray(left) ? left : []), ...(Array.isArray(right) ? right : [])].forEach((item) => {
    const key = conflictConceptLabel(item?.concept, item?.relatedConcepts);
    if (!key || seen.has(key)) return;
    seen.add(key);
    merged.push(item);
  });
  return merged.slice(0, 6);
}

function conflictFamilyForConcept(concept) {
  if (concept === "主图" || concept === "点击率") return "visual_click";
  return String(concept || "");
}

function conflictSourceSignature(sourceIndexes = []) {
  return uniqueOrdered(sourceIndexes.filter((value) => Number.isInteger(value)), 8)
    .slice()
    .sort((a, b) => a - b)
    .join("|");
}

function preferredConflictConcept(concepts = [], family = "") {
  if (family === "visual_click") {
    if (concepts.includes("主图")) return "主图";
    if (concepts.includes("点击率")) return "点击率";
  }
  return concepts[0] || "";
}

function mergeConflictItems(left = [], right = [], limit = 2) {
  const seen = new Set();
  const items = [];
  [...(Array.isArray(left) ? left : []), ...(Array.isArray(right) ? right : [])].forEach((item) => {
    const key = `${Number.isInteger(item?.sourceIndex) ? item.sourceIndex : ""}|${item?.title || ""}|${item?.quote || ""}`;
    if (seen.has(key)) return;
    seen.add(key);
    items.push(item);
  });
  return items.slice(0, limit);
}

function conflictRepresentative(items = []) {
  return items.find((item) => Number.isInteger(item?.sourceIndex)) || items[0] || {};
}

function buildConflictComparison(concept, support, caution, relatedConcepts = [], supportingReasons = []) {
  const conceptLabel = conflictConceptLabel(concept, relatedConcepts);
  const supportSource = conflictSourceSummary(support);
  const cautionSource = conflictSourceSummary(caution);
  const supportQuote = String(support?.quote || "").trim();
  const cautionQuote = String(caution?.quote || "").trim();
  return {
    summary: `${conceptLabel}相关资料出现了优先级分歧：一侧强调先处理，另一侧提醒先看其他瓶颈。`,
    differenceFocus: `${conceptLabel}是不是当前第一优先级：一边强调先处理，另一边提醒先确认其他瓶颈。`,
    supportLabel: "支持先处理",
    supportQuote,
    supportSource,
    cautionLabel: "提醒先复核",
    cautionQuote,
    cautionSource,
    suggestedCheck: conflictSuggestedCheck(concept),
    nextQuestion: buildConflictNextQuestion(concept, supportSource, cautionSource, supportQuote, cautionQuote, relatedConcepts, supportingReasons),
  };
}

function buildConflictNextQuestion(concept, supportSource, cautionSource, supportQuote, cautionQuote, relatedConcepts = [], supportingReasons = []) {
  const conceptLabel = conflictConceptLabel(concept, relatedConcepts);
  const requiredData = conflictRequiredData(concept, relatedConcepts, supportingReasons);
  const labels = requiredData.map((item) => item.label).join("、");
  return {
    intent: "resolve_conflict",
    question: [
      `围绕“${conceptLabel}是不是当前第一优先级”继续分析。`,
      `一边认为：${supportQuote || `${conceptLabel}应该先处理。`}`,
      `另一边认为：${cautionQuote || `${conceptLabel}需要先复核其他瓶颈。`}`,
      "请结合我的具体产品数据判断下一步优先级，并说明当前更适合采纳哪一侧观点。",
      `需要我补充的数据项：${labels}。`,
      "数据用途：",
      ...requiredData.map((item) => `- ${item.label}：${item.verifies || item.reason}`),
      "我的数据：",
      ...requiredData.map((item) => `- ${item.label}：`),
    ].join("\n"),
    requiredData,
    evidenceRefs: {
      supportSourceIndex: supportSource.sourceIndex,
      cautionSourceIndex: cautionSource.sourceIndex,
    },
    boundary: "如果数据不完整，只能列出还缺什么，不能直接判断哪一侧正确。",
  };
}

function conflictRequiredData(concept, relatedConcepts = [], supportingReasons = []) {
  const common = [
    { id: "ctr", label: "主图点击率 / 广告 CTR", reason: "判断点击入口是否是瓶颈" },
    { id: "cvr", label: "转化率 / CVR", reason: "判断问题是否进入页面承接" },
    { id: "price", label: "价格与同屏竞品价格带", reason: "排除价格导致的转化问题" },
    { id: "reviews", label: "评分、评价数、主要差评", reason: "排除信任基础问题" },
    { id: "traffic", label: "核心词曝光、点击、广告搜索词", reason: "判断流量是否足够有效" },
  ];
  const extras = {
    主图: [
      { id: "image_competition", label: "搜索结果同屏竞品主图差异", reason: "判断主图是否真的弱于竞品" },
      { id: "mobile_image", label: "手机端首屏主图表现", reason: "确认移动端点击入口是否清楚" },
    ],
    点击率: [
      { id: "image_competition", label: "搜索结果同屏竞品主图差异", reason: "判断点击差距来源" },
      { id: "keyword_rank", label: "核心词排名位置", reason: "排除位置变化导致的点击率波动" },
    ],
    转化率: [
      { id: "listing_content", label: "副图、五点、A+ 页面承接", reason: "判断页面是否解释清楚购买理由" },
      { id: "buybox_offer", label: "优惠、配送、库存和 Buy Box 状态", reason: "排除成交条件问题" },
    ],
    Listing: [
      { id: "listing_content", label: "标题、五点、副图、A+ 页面承接", reason: "判断页面内容是否承接关键词和卖点" },
      { id: "keyword_fit", label: "核心关键词与页面卖点匹配度", reason: "确认流量和页面是否一致" },
    ],
    广告: [
      { id: "acos", label: "ACOS、CPC、广告花费和订单", reason: "判断广告是在放大问题还是带来有效词" },
      { id: "search_terms", label: "广告搜索词相关性", reason: "确认广告流量是否精准" },
    ],
    价格: [
      { id: "margin", label: "毛利空间和可接受优惠幅度", reason: "判断价格动作是否可持续" },
      { id: "coupon", label: "优惠券、折扣和竞品促销", reason: "确认同屏价格感知差异" },
    ],
    关键词: [
      { id: "indexing", label: "核心词收录和自然排名", reason: "确认词库是否真的进入搜索结果" },
      { id: "keyword_intent", label: "关键词购买意图和页面承接", reason: "排除词不准导致的低效流量" },
    ],
    评价: [
      { id: "review_gap", label: "同屏竞品评分和评价数量差距", reason: "判断信任基础差距" },
      { id: "negative_reviews", label: "差评主题和近期新增差评", reason: "定位转化阻力" },
    ],
  };
  const concepts = uniqueOrdered([concept, ...(Array.isArray(relatedConcepts) ? relatedConcepts : [])], 8);
  const merged = [...common, ...concepts.flatMap((item) => extras[item] || [])];
  const seen = new Set();
  return merged.filter((item) => {
    if (seen.has(item.id)) return false;
    seen.add(item.id);
    return true;
  }).slice(0, 7).map((item) => enrichConflictRequiredData(item, concept, relatedConcepts, supportingReasons));
}

function enrichConflictRequiredData(item, concept, relatedConcepts = [], supportingReasons = []) {
  const conceptLabel = conflictConceptLabel(concept, relatedConcepts);
  const supportingConcepts = uniqueOrdered((Array.isArray(supportingReasons) ? supportingReasons : []).flatMap((reason) => [reason.concept, ...(reason.relatedConcepts || [])]).filter(Boolean), 8);
  const targetRole = supportingDataIds(supportingConcepts).has(item.id) ? "supporting" : "primary";
  return {
    ...item,
    targetRole,
    verifies: conflictRequiredDataVerification(item, conceptLabel, targetRole, supportingConcepts),
  };
}

function supportingDataIds(supportingConcepts = []) {
  const ids = new Set();
  if (supportingConcepts.includes("转化率") || supportingConcepts.includes("Listing")) {
    ["cvr", "price", "reviews", "listing_content", "buybox_offer"].forEach((id) => ids.add(id));
  }
  if (supportingConcepts.includes("价格")) ids.add("price");
  if (supportingConcepts.includes("评价")) ids.add("reviews");
  return ids;
}

function conflictRequiredDataVerification(item, conceptLabel, targetRole, supportingConcepts = []) {
  if (targetRole === "supporting") {
    if (item.id === "cvr") return "验证转化率是否只是辅助因素，还是已经进入页面承接问题";
    if (item.id === "price") return "验证价格带是否在拖累转化判断";
    if (item.id === "reviews") return "验证评分、评价数和差评是否构成信任阻力";
    if (item.id === "listing_content") return "验证页面承接是否比点击入口更值得先处理";
    if (item.id === "buybox_offer") return "验证成交条件是否干扰转化判断";
    return `验证${supportingConcepts.join("/") || "辅助因素"}是否影响主判断`;
  }
  if (item.id === "ctr") return `验证${conceptLabel}是否是点击入口瓶颈`;
  if (item.id === "traffic") return "验证是不是曝光位置、流量质量或广告搜索词导致的问题";
  if (item.id === "image_competition") return "验证主图是否弱于搜索结果同屏竞品";
  if (item.id === "mobile_image") return "验证手机端首屏主图是否影响点击";
  if (item.id === "keyword_rank") return "验证核心词排名位置是否导致点击率波动";
  return `验证${conceptLabel}是否应作为当前第一优先级`;
}

function conflictSignalLabel(signal = {}) {
  return conflictConceptLabel(signal.concept, signal.relatedConcepts);
}

function conflictConceptLabel(concept, relatedConcepts = []) {
  return uniqueOrdered([concept, ...(Array.isArray(relatedConcepts) ? relatedConcepts : [])].filter(Boolean), 4).join("/");
}

function conflictSourceSummary(item = {}) {
  return {
    sourceIndex: Number.isInteger(item.sourceIndex) ? item.sourceIndex : undefined,
    author: String(item.author || ""),
    title: String(item.title || ""),
    date: String(item.date || ""),
    sourceUrl: String(item.sourceUrl || ""),
    sourcePath: String(item.sourcePath || ""),
  };
}

function conflictSuggestedCheck(concept) {
  if (concept === "主图" || concept === "点击率") return "先看你的主图点击率、核心词曝光、搜索结果同屏竞品和广告点击数据，再决定是否先改主图。";
  if (concept === "转化率" || concept === "Listing") return "先看点击率是否充足，再对比评价、价格、页面承接、副图和五点，判断瓶颈是不是页面转化。";
  if (concept === "广告") return "先看自然流量、广告点击量、搜索词相关性、ACOS 和转化率，判断广告是在放大问题还是带来有效词。";
  if (concept === "价格") return "先比较同屏竞品价格带、优惠力度、毛利空间和转化率变化，再决定是否用价格解决问题。";
  if (concept === "关键词") return "先检查核心词搜索量、收录、广告搜索词和页面承接，再决定是改词库还是改页面。";
  if (concept === "评价") return "先对比评分、差评主题、评价数量和同屏竞品，再判断评价是不是当前最大瓶颈。";
  return "先把你的产品数据和页面材料补齐，再判断哪一侧观点更适合当前情况。";
}

function conflictConceptsForText(text) {
  const value = String(text || "");
  const concepts = [
    ["主图", /主图|首图|图片|视觉/],
    ["广告", /广告|SP|SBV|投放/],
    ["价格", /价格|售价|优惠|折扣/],
    ["关键词", /关键词|搜索词|词库|流量词/],
    ["Listing", /Listing|标题|五点|页面|A\\+/i],
    ["评价", /评价|Review|星级|评分/i],
    ["转化率", /转化率|CVR|转化/i],
    ["点击率", /点击率|CTR|点击/i],
  ];
  return concepts.filter(([, pattern]) => pattern.test(value)).map(([concept]) => concept);
}

function conflictStanceForText(text, concept) {
  const value = String(text || "").replace(/\s+/g, "");
  const boundaryPattern = /不要只|不能只|不等于|不是救命稻草|还要|也要|同时|不仅|不只是/;
  const strongSupportPattern = /唯一|必须先|必须优先|只要|第一优先级|核心瓶颈|先.*(改|做|优化|检查)|优先.*(改|做|优化|检查)/;
  const supportPattern = /决定|极大程度|核心|关键|优先|必须|需要|应该|先|提升|优化|影响|取决于|唯一|重要|放大器/;
  const cautionPattern = /不建议先|不用先|先别急着|优先级不高|不是当前瓶颈|不是.*(关键|重点|优先|核心|瓶颈)|不要.*(先|优先|只|仅)|不能.*(先|优先|只|靠)|没必要|无意义|先放一放|暂缓|低优先级|救命稻草/;
  const hasSupport = supportPattern.test(value);
  const hasCaution = cautionPattern.test(value) && cautionTargetsConcept(value, concept);
  if (!hasSupport && !hasCaution) return "";
  if (boundaryPattern.test(value) && !strongSupportPattern.test(value)) return "";
  if (hasSupport && hasCaution) {
    return cautionPattern.test(value) ? "caution" : "";
  }
  return hasCaution ? "caution" : "support";
}

function cautionTargetsConcept(value, concept) {
  const target = String(concept || "").replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  if (!target) return false;
  const near = `.{0,14}`;
  const beforeTarget = "(不建议先|不用先|先别急着|不要.{0,8}(先|优先|只|仅)|不能.{0,8}(先|优先|只|靠))";
  const afterTarget = "(优先级不高|不是当前瓶颈|不是.{0,8}(关键|重点|优先|核心|瓶颈)|没必要|无意义|先放一放|暂缓|低优先级|救命稻草)";
  return new RegExp(`${beforeTarget}${near}${target}|${target}${near}${afterTarget}`).test(value);
}

function parseSourceDate(value) {
  const text = String(value || "").trim();
  if (!text) return null;
  const match = text.match(/(\d{4})[-/.年](\d{1,2})[-/.月](\d{1,2})?/);
  if (!match) return null;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3] || 1);
  const parsed = new Date(Date.UTC(year, month - 1, day));
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}

function uniqueOrdered(items, limit = 20) {
  const seen = new Set();
  const result = [];
  for (const item of Array.isArray(items) ? items : []) {
    const key = String(item);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    result.push(item);
    if (result.length >= limit) break;
  }
  return result;
}

function evidenceAuditLabel(level) {
  if (level === "high") return "引用支撑较充分";
  if (level === "medium") return "需要人工复核";
  return "需要重查";
}

function evidenceAuditSummary(level, sourceEvidenceCount, unsupportedCount, conflictCount = 0) {
  if (conflictCount > 0) return `来源中存在 ${conflictCount} 个可能相反的观点，需要人工对比后再采纳结论。`;
  if (level === "high") return `这次回答有 ${sourceEvidenceCount} 条原文证据支撑，推断边界较清楚。`;
  if (sourceEvidenceCount === 0) return "这次回答缺少可直接定位的原文证据，需要先补资料或换更具体的问题。";
  if (level === "low") return `这次回答只有有限原文证据，还包含 ${unsupportedCount} 条推断或行动建议，建议重查或补充限定条件。`;
  return `这次回答有 ${sourceEvidenceCount} 条原文证据，但还有 ${unsupportedCount} 条推断或行动建议需要你用产品数据验证。`;
}

function evidenceConceptMatches(text, concepts) {
  return concepts.filter((concept) => textMentionsConcept(text, concept)).length;
}

function buildRankedEvidence(ranked, articles, options = {}) {
  const articleKeys = new Map(articles.map((article, index) => [sourceKey(article), index]));
  const mapped = ranked
    .map((item, index) => {
      const sourceIndex = articleKeys.get(sourceKey(item.source));
      if (!Number.isInteger(sourceIndex)) return null;
      return {
        evidenceIndex: index,
        sourceIndex,
        quote: item.text,
        score: item.score,
        author: item.source.author,
        title: item.source.title,
        date: item.source.date,
      };
    })
    .filter(Boolean);

  if (options.diversifyAuthors) return diversifyEvidenceByAuthor(mapped, 8);
  return mapped.slice(0, 8);
}

function diversifyEvidenceByAuthor(items = [], limit = 8) {
  const rows = Array.isArray(items) ? items.filter(Boolean) : [];
  const selected = [];
  const seen = new Set();
  const addItem = (item) => {
    if (!item) return false;
    const key = `${item.sourceIndex}|${String(item.quote || "").slice(0, 80)}`;
    if (seen.has(key)) return false;
    seen.add(key);
    selected.push(item);
    return true;
  };
  const authors = uniqueOrdered(rows.map((item) => item.author).filter(Boolean), limit);
  const orderedAuthors = [
    ...AUTHORS.filter((author) => authors.includes(author)),
    ...authors.filter((author) => !AUTHORS.includes(author)),
  ];

  for (const author of orderedAuthors) {
    addItem(rows.find((item) => item.author === author));
    if (selected.length >= limit) return selected;
  }
  for (const item of rows) {
    addItem(item);
    if (selected.length >= limit) return selected;
  }
  return selected;
}

function ensureAuthorEvidenceCoverage(items = [], articles = [], question = "", limit = 8) {
  const rows = Array.isArray(items) ? items.filter(Boolean) : [];
  const evidenceAuthors = new Set(rows.map((item) => item.author).filter(Boolean));
  const profile = buildQuestionProfile(question);
  const additions = [];

  articles.forEach((article, sourceIndex) => {
    const author = String(article?.author || "").trim();
    if (!author || evidenceAuthors.has(author)) return;
    const sentence = splitSentences(article.body)
      .map((text) => ({ text, score: scoreText(text, profile) }))
      .filter((item) => item.text.trim())
      .sort((a, b) => b.score - a.score || b.text.length - a.text.length)[0];
    if (!sentence || Number(sentence.score || 0) <= 0) return;
    const quote = sentence?.text || article.excerpt || article.body || "";
    if (!String(quote || "").trim()) return;
    additions.push({
      evidenceIndex: rows.length + additions.length,
      sourceIndex,
      quote: compactGraphLabel(quote, 280),
      score: sentence?.score || 0,
      author,
      title: article.title,
      date: article.date,
    });
    evidenceAuthors.add(author);
  });

  return diversifyEvidenceByAuthor([...rows, ...additions], limit);
}

function sourceExcerptForArticle(article, evidenceItems = []) {
  const base = String(article?.excerpt || article?.body || "").trim();
  const body = String(article?.body || article?.excerpt || "");
  const quoted = uniqueOrdered(
    evidenceItems
      .map((item) => String(item.quote || "").trim())
      .filter((quote) => quote && body.includes(quote)),
    2,
  );
  if (quoted.length === 0) return base;
  const normalizedBase = normalizeEvidenceText(base.slice(0, 500));
  if (quoted.every((quote) => normalizedBase.includes(normalizeEvidenceText(quote)))) return base;
  return `${quoted.join("\n")}\n${base}`.slice(0, 700);
}

function normalizeEvidenceText(value) {
  return String(value || "")
    .replace(/[【】\[\]（）()《》「」“”"'`]/g, "")
    .replace(/\s+/g, "")
    .trim();
}

export function buildAnswerGraph(question, answer, sources = [], retrievalQuestion = question, rankedEvidence = [], evidenceChain = undefined) {
  const nodes = [];
  const edges = [];
  const seenNodes = new Set();
  const seenEdges = new Set();

  const addNode = (node) => {
    if (!node.id || seenNodes.has(node.id)) return;
    seenNodes.add(node.id);
    nodes.push(node);
  };
  const addEdge = (edge) => {
    if (!seenNodes.has(edge.from) || !seenNodes.has(edge.to)) return;
    const key = `${edge.from}->${edge.to}:${edge.type}`;
    if (seenEdges.has(key)) return;
    seenEdges.add(key);
    edges.push(edge);
  };

  addNode({
    id: "question",
    type: "question",
    label: compactGraphLabel(question || "本轮问题", 34),
    detail: String(question || ""),
    prompt: buildGraphPrompt("question", question || "本轮问题"),
  });

  const sections = extractAnswerSections(answer);
  sections.conclusions.slice(0, 3).forEach((text, index) => {
    const id = `point:${index}`;
    addNode({
      id,
      type: "point",
      label: compactGraphLabel(text, 30),
      detail: text,
      claimId: `system-inference:${index}`,
      identity: "二次摘要/系统整理",
      canUseAsEvidence: false,
      prompt: buildGraphPrompt("point", text),
    });
    addEdge({ from: "question", to: id, type: "answers" });
  });

  sections.steps.slice(0, 4).forEach((text, index) => {
    const id = `step:${index}`;
    addNode({
      id,
      type: "step",
      label: compactGraphLabel(text, 22),
      detail: text,
      claimId: `action-advice:${index}`,
      identity: "二次摘要/系统整理",
      canUseAsEvidence: false,
      prompt: buildGraphPrompt("step", text),
    });
    addEdge({ from: "question", to: id, type: "next_step" });
  });

  const conceptText = [retrievalQuestion, question, answer, ...sources.map((source) => `${source.title}\n${source.excerpt}`)].join("\n");
  const concepts = detectGraphConcepts(conceptText).slice(0, 8);
  concepts.forEach((label) => {
    const id = `concept:${label}`;
    addNode({ id, type: "concept", label, prompt: buildGraphPrompt("concept", label) });
    addEdge({ from: "question", to: id, type: "mentions" });
  });

  for (const node of nodes.filter((item) => item.type === "point" || item.type === "step")) {
    for (const concept of concepts) {
      if (!textMentionsConcept(node.detail || node.label, concept)) continue;
      addEdge({ from: node.id, to: `concept:${concept}`, type: "contains" });
    }
  }

  if (sources.length === 0) {
    addNode({
      id: "empty:sources",
      type: "empty",
      label: "暂无来源支撑",
      prompt: "请换一种更具体的问法重新检索作者资料，并说明这次缺少哪些来源支撑。",
    });
    addEdge({ from: "question", to: "empty:sources", type: "no_source" });
    return { nodes, edges };
  }

  sources.slice(0, 5).forEach((source, index) => {
    const sourceId = `source:${index}`;
    addNode({
      id: sourceId,
      type: "source",
      label: compactGraphLabel(source.title || `来源 ${index + 1}`, 24),
      detail: source.excerpt || "",
      sourceIndex: index,
      identity: "候选作者原文来源",
      canUseAsEvidence: false,
    });

    const sourceText = `${source.title}\n${source.excerpt}`;
    let linkedConcept = false;
    for (const concept of concepts) {
      if (!textMentionsConcept(sourceText, concept)) continue;
      addEdge({ from: `concept:${concept}`, to: sourceId, type: "related_source", strength: "related", label: "关键词相关" });
      linkedConcept = true;
    }
    if (!linkedConcept) addEdge({ from: "question", to: sourceId, type: "related_source", strength: "related", label: "检索相关" });

    if (source.author) {
      const authorId = `author:${source.author}`;
      addNode({ id: authorId, type: "author", label: source.author });
      addEdge({ from: sourceId, to: authorId, type: "written_by" });
    }
  });

  const evidence = Array.isArray(rankedEvidence) ? rankedEvidence.filter((item) => Number.isInteger(item.sourceIndex)) : [];
  const evidenceNodeIdsBySourceIndex = new Map();
  const sourceEvidenceClaims = Array.isArray(evidenceChain?.claims)
    ? evidenceChain.claims.filter((claim) => claim?.type === "source_evidence" && Number.isInteger(claim.sourceIndex)).slice(0, 5)
    : [];
  sourceEvidenceClaims.forEach((claim, index) => {
    const nodeId = `evidence:${safeGraphId(claim.id || index)}`;
    addNode({
      id: nodeId,
      type: "evidence",
      label: compactGraphLabel(claim.quote || claim.text || `证据 ${index + 1}`, 26),
      detail: claim.quote || claim.text || "",
      sourceIndex: claim.sourceIndex,
      claimId: claim.id,
      identity: claim.trustLabel || "作者原文证据",
      canUseAsEvidence: claim.canUseAsEvidence === true,
    });
    if (!evidenceNodeIdsBySourceIndex.has(claim.sourceIndex)) evidenceNodeIdsBySourceIndex.set(claim.sourceIndex, []);
    evidenceNodeIdsBySourceIndex.get(claim.sourceIndex).push(nodeId);
    addEdge({ from: "question", to: nodeId, type: "evidence_found", strength: "evidence", label: "命中原文" });
    addEdge({ from: nodeId, to: `source:${claim.sourceIndex}`, type: "quoted_from", strength: "evidence", label: "来自来源" });
  });

  for (const node of nodes.filter((item) => item.type === "point" || item.type === "step")) {
    relatedEvidenceIndexes(node.detail || node.label, evidence)
      .slice(0, 2)
      .forEach((sourceIndex) => {
        const targetId = evidenceNodeIdsBySourceIndex.get(sourceIndex)?.[0] || `source:${sourceIndex}`;
        addEdge({
          from: node.id,
          to: targetId,
          type: "supported_by",
          strength: "evidence",
          label: "原文证据",
        });
      });
  }

  return { nodes, edges };
}

function buildGraphPrompt(type, text) {
  const label = compactGraphLabel(sanitizeGraphPromptText(text || "这个节点") || "这个节点", 160);
  if (type === "question") {
    return `请围绕本轮问题「${label}」继续拆成学习路线，并区分作者原文、系统整理和我的业务验证。`;
  }
  if (type === "point") {
    return `请围绕这个答案要点继续展开：「${label}」。请说明它由哪些作者原文支撑、哪些只是系统推断、下一步应该怎么验证。`;
  }
  if (type === "step") {
    return `我准备执行这一步：「${label}」。请基于本轮来源拆成低风险检查清单，并说明需要补哪些产品或广告数据。`;
  }
  if (type === "concept") {
    return `请围绕本轮图谱概念「${label}」继续学习：它和本轮问题有什么关系、有哪些来源支撑、下一步我该如何验证。`;
  }
  return `请围绕「${label}」继续追问，并标出来源边界。`;
}

function sanitizeGraphPromptText(value) {
  return redactBusinessFactsForRetrieval(String(value || ""))
    .replace(/\bB0[A-Z0-9]{8}\b/gi, "ASIN")
    .replace(/\bASIN\s+ASIN\b/gi, "ASIN")
    .replace(/\s+/g, " ")
    .trim();
}

export function buildLearningCard(question, answer, sources = [], retrievalQuestion = question) {
  const type = detectAnswerType(`${retrievalQuestion}\n${question}`);
  const sections = extractAnswerSections(answer);
  const conclusions = sections.conclusions.slice(0, 3);
  const nextActions = (sections.steps.length > 0 ? sections.steps : buildExecutionSteps(retrievalQuestion || question)).slice(0, 5);
  const missingInputs = buildMissingInputs(type, sources);
  const evidence = sources.slice(0, 5).map((source, index) => ({
    sourceIndex: index,
    title: source.title || `来源 ${index + 1}`,
    author: source.author || "",
    date: source.date || "",
  }));

  return {
    intent: describeIntent(type),
    takeaway: conclusions[0] || buildFallbackTakeaway(type),
    conclusions,
    nextActions,
    missingInputs,
    followUps: buildFollowUps(type),
    evidence,
    studyChecks: sources.length > 0
      ? buildStudyChecks({
          type,
          question,
          retrievalQuestion,
          takeaway: conclusions[0] || buildFallbackTakeaway(type),
          conclusions,
          nextActions,
          missingInputs,
          sources,
        })
      : [],
  };
}

export function buildSourceStudyPack(input = {}) {
  const question = String(input.question || "").trim();
  const retrievalQuestion = String(input.retrievalQuestion || question).trim();
  const answer = String(input.answer || "");
  const sources = Array.isArray(input.sources) ? input.sources : [];
  const claims = Array.isArray(input.evidenceChain?.claims) ? input.evidenceChain.claims : [];
  const sourceClaims = claims
    .filter((claim) => claim?.type === "source_evidence" && claim.id && Number.isInteger(claim.sourceIndex))
    .filter((claim) => sources[claim.sourceIndex])
    .slice(0, 5);

  const baseBoundary = "研读包只展示候选学习材料；只有被你确认有用的作者原文证据，才能进入正式学习档案和后续追问。系统整理、抽认卡、缺口清单都不是新的作者证据。";
  if (sourceClaims.length === 0) {
    return {
      title: "本轮来源研读包",
      status: "needs_source",
      boundary: "当前没有可绑定的作者原文证据，不能生成正式研读包；请先补来源或重新提问。",
      sourceCards: [],
      concepts: [],
      flashcards: [],
      gaps: [
        {
          id: "gap:source",
          label: "缺少作者原文来源",
          reason: "这轮回答没有可定位的作者原文证据，不能沉淀成学习结论。",
          prompt: `请围绕「${compactGraphLabel(retrievalQuestion || question || "这个问题", 80)}」重新检索作者原文，并优先给出可引用片段。`,
        },
      ],
      prompts: [
        `请重新检索「${compactGraphLabel(retrievalQuestion || question || "这个问题", 80)}」的作者原文证据。`,
      ],
    };
  }

  const sourceCards = sourceClaims.map((claim, index) => {
    const source = sources[claim.sourceIndex] || {};
    const quote = compactGraphLabel(claim.quote || claim.text || source.excerpt || "", 260);
    const label = compactGraphLabel(claim.text || quote || source.title || `来源 ${index + 1}`, 150);
    return {
      id: `study-source:${index}`,
      claimId: claim.id,
      sourceIndex: claim.sourceIndex,
      identity: "作者原文",
      canUseAsEvidence: true,
      evidenceKind: "source_evidence",
      label,
      title: source.title || claim.title || `来源 ${claim.sourceIndex + 1}`,
      author: source.author || claim.author || "",
      date: source.date || claim.date || "",
      sourceUrl: source.sourceUrl || "",
      sourcePath: source.sourcePath || "",
      quote,
      why: "先读这条原文，再判断回答里的整理是否适合你的产品场景。",
      prompt: `请只基于这条作者原文继续解释：${quote}`,
    };
  });

  const conceptText = [
    retrievalQuestion,
    question,
    answer,
    ...sourceCards.map((card) => `${card.label}\n${card.quote}`),
  ].join("\n");
  const concepts = detectGraphConcepts(conceptText)
    .slice(0, 6)
    .map((label, index) => {
      const related = sourceCards
        .filter((card) => textMentionsConcept(`${card.label}\n${card.quote}\n${card.title}`, label))
        .map((card) => card.claimId);
      return {
        id: `study-concept:${index}`,
        label,
        identity: "系统整理",
        canUseAsEvidence: false,
        sourceClaimIds: (related.length > 0 ? related : sourceCards.slice(0, 2).map((card) => card.claimId)).slice(0, 3),
        prompt: `请围绕「${label}」复盘本轮来源，并区分作者原文和系统整理。`,
      };
    });

  const flashcards = sourceCards.slice(0, 4).map((card, index) => ({
    id: `study-flashcard:${index}`,
    claimId: card.claimId,
    sourceIndex: card.sourceIndex,
    identity: "系统整理",
    canUseAsEvidence: false,
    question: index === 0 ? "这条作者原文最先提醒我看什么？" : "这条来源能支撑哪一个学习点？",
    answer: card.quote,
    boundary: "这张卡是系统整理的复习提示，不是新的作者证据；要采纳结论仍需回到对应原文。",
    prompt: `请把这条来源整理成一个可执行的亚马逊检查动作：${card.quote}`,
  }));

  const missingInputs = Array.isArray(input.learningCard?.missingInputs) ? input.learningCard.missingInputs : [];
  const gaps = missingInputs.slice(0, 4).map((item, index) => ({
    id: `gap:${index}`,
    label: compactGraphLabel(item, 80),
    reason: "本轮来源没有覆盖这个业务前提，后续判断需要你补数据或指定场景。",
    prompt: `围绕「${compactGraphLabel(item, 80)}」我应该补哪些亚马逊业务材料？`,
  }));
  if (sourceCards.length < 2) {
    gaps.unshift({
      id: "gap:independent-source",
      label: "独立来源不足",
      reason: "当前可读来源少于 2 条，不能把单一来源当成多方验证。",
      prompt: "请继续找能独立支持或反驳本轮判断的作者原文。",
    });
  }

  return {
    title: "本轮来源研读包",
    status: "needs_review",
    boundary: baseBoundary,
    sourceCards,
    concepts,
    flashcards,
    gaps: gaps.slice(0, 5),
    prompts: [
      "请按阅读顺序讲解本轮来源，并指出哪些结论还不能下定论。",
      "请把本轮来源转成我的 Listing 检查清单，但不要把系统整理当作者证据。",
      "请列出本轮还缺哪些产品、广告、关键词或评价数据。",
    ],
  };
}

export function buildKnowledgeGapRadar(input = {}) {
  const question = String(input.question || "").trim();
  const retrievalQuestion = String(input.retrievalQuestion || question).trim();
  const sources = Array.isArray(input.sources) ? input.sources : [];
  const claims = Array.isArray(input.evidenceChain?.claims) ? input.evidenceChain.claims : [];
  const sourceClaims = claims
    .filter((claim) => claim?.type === "source_evidence" && claim?.trustKind === "author_original")
    .filter((claim) => Number.isInteger(claim.sourceIndex) && sources[claim.sourceIndex]);
  const authorCount = uniqueOrdered(sources.map((source) => source.author).filter(Boolean), 12).length;
  const sourceIndexes = uniqueOrdered(sourceClaims.map((claim) => claim.sourceIndex), 8);
  const claimIds = uniqueOrdered(sourceClaims.map((claim) => claim.id).filter(Boolean), 8);
  const conflictSignals = Array.isArray(input.evidenceAudit?.conflictSignals) ? input.evidenceAudit.conflictSignals : [];
  const auditCounts = input.evidenceAudit?.counts && typeof input.evidenceAudit.counts === "object" ? input.evidenceAudit.counts : {};
  const dataRequests = Array.isArray(input.validationPack?.dataRequests) ? input.validationPack.dataRequests : [];
  const missingInputs = Array.isArray(input.learningCard?.missingInputs) ? input.learningCard.missingInputs : [];
  const studyGaps = Array.isArray(input.sourceStudyPack?.gaps) ? input.sourceStudyPack.gaps : [];
  const missingDataLabels = uniqueOrdered([
    ...dataRequests.map((item) => item?.label).filter(Boolean),
    ...missingInputs.filter(Boolean),
  ], 6);
  const unsupportedCount = Number(auditCounts.needsSource || 0)
    + Number(auditCounts.systemInferences || 0)
    + Number(auditCounts.actionAdvice || 0);
  const topic = compactGraphLabel(retrievalQuestion || question || "本轮问题", 90);
  const gaps = [];
  const addGap = (id, kind, label, reason, prompt, extra = {}) => {
    if (gaps.some((item) => item.id === id || item.label === label)) return;
    gaps.push({
      id,
      kind,
      label: compactGraphLabel(label, 90),
      reason: compactGraphLabel(reason, 220),
      prompt: compactGraphLabel(prompt, 420),
      sourceIndexes: Array.isArray(extra.sourceIndexes) ? uniqueOrdered(extra.sourceIndexes.filter(Number.isInteger), 8) : [],
      claimIds: Array.isArray(extra.claimIds) ? uniqueOrdered(extra.claimIds.map((idValue) => String(idValue || "")).filter(Boolean), 8) : [],
      canUseAsEvidence: false,
    });
  };

  if (sourceClaims.length === 0) {
    const sourceGap = studyGaps.find((item) => /来源|作者原文/.test(`${item?.label || ""}\n${item?.reason || ""}`));
    addGap(
      "gap:source",
      "source",
      "缺少作者原文支撑",
      sourceGap?.reason || "这轮回答没有可定位的作者原文证据，不能沉淀为可靠学习结论。",
      sourceGap?.prompt || `请围绕「${topic}」重新检索作者原文，并优先给出可引用片段。`,
    );
  }

  if (conflictSignals.length > 0) {
    const relatedSources = uniqueOrdered(conflictSignals.flatMap((item) => Array.isArray(item?.sourceIndexes) ? item.sourceIndexes : []), 8);
    const labels = conflictSignals.map(conflictSignalLabel).filter(Boolean).slice(0, 3).join("、") || "本轮关键判断";
    addGap(
      "gap:conflict",
      "conflict",
      "来源观点需要对比",
      `轻量扫描发现 ${conflictSignals.length} 个可能冲突点：${labels}。`,
      `请对比本轮冲突来源，说明「${labels}」哪些是作者原文支持，哪些只是适用场景不同。`,
      { sourceIndexes: relatedSources },
    );
  }

  if (sourceClaims.length > 0 && sourceIndexes.length < 2) {
    addGap(
      "gap:independent-source",
      "source",
      "独立来源不足",
      "当前可引用证据集中在少数来源，不能当成多方验证。",
      `请继续找能独立支持或反驳「${topic}」的作者原文。`,
      { sourceIndexes, claimIds },
    );
  }

  if (sourceClaims.length > 0 && authorCount < 2) {
    addGap(
      "gap:author-view",
      "source",
      "作者视角不足",
      "本轮来源作者过少，可能只覆盖一种运营经验或场景。",
      `请找另一位作者对「${topic}」的看法，并明确相同点和不同适用条件。`,
      { sourceIndexes, claimIds },
    );
  }

  if (missingDataLabels.length > 0) {
    addGap(
      "gap:business-data",
      "data",
      "缺少你的产品数据",
      `下一步判断还需要：${missingDataLabels.slice(0, 4).join("、")}。`,
      `我补充这些数据：${missingDataLabels.slice(0, 4).join("、")}。请结合本轮作者来源判断下一步优先级。`,
      { sourceIndexes, claimIds },
    );
  }

  if (unsupportedCount > 0 && sourceClaims.length > 0) {
    addGap(
      "gap:claim-boundary",
      "boundary",
      "系统整理需要复核",
      "回答里包含系统整理或行动建议，不能直接当作者原文结论。",
      "请把本轮回答逐条分成作者原文、系统整理、需要我的业务数据验证三类。",
      { sourceIndexes, claimIds },
    );
  }

  if (gaps.length === 0) {
    addGap(
      "gap:validate",
      "validation",
      "进入小范围验证",
      "本轮已有可用来源和基本边界，下一步应转向真实业务验证。",
      input.validationPack?.followUpPrompt || `请把「${topic}」转成 7 天小实验和复盘表。`,
      { sourceIndexes, claimIds },
    );
  }

  const status = sourceClaims.length === 0
    ? "needs_source"
    : conflictSignals.length > 0
      ? "needs_review"
      : missingDataLabels.length > 0
        ? "needs_data"
        : (sourceIndexes.length < 2 || authorCount < 2)
          ? "needs_review"
          : "ready_to_validate";
  const priority = gaps[0];

  return {
    title: "知识缺口雷达",
    status,
    summary: knowledgeGapRadarSummary(status, priority, {
      sourceCount: sources.length,
      evidenceCount: sourceClaims.length,
      authorCount,
    }),
    priority,
    gaps: gaps.slice(0, 6),
    metrics: {
      sourceCount: sources.length,
      evidenceCount: sourceClaims.length,
      authorCount,
      conflictCount: conflictSignals.length,
      missingDataCount: missingDataLabels.length,
      unsupportedCount,
    },
    boundary: "知识缺口雷达只决定下一步补资料、补数据或复核路径，不改变作者原文证据边界，也不会把系统整理变成新证据。",
  };
}

function knowledgeGapRadarSummary(status, priority, metrics = {}) {
  if (status === "needs_source") return "先补作者原文来源，再谈结论沉淀。";
  if (status === "needs_review") return `先处理「${priority?.label || "来源复核"}」，再决定是否执行。`;
  if (status === "needs_data") return `本轮已有 ${metrics.evidenceCount || 0} 条作者证据，下一步要补你的产品数据。`;
  if (status === "ready_to_validate") return "本轮来源边界较清楚，下一步进入小范围业务验证。";
  return "先看优先级最高的缺口，再继续追问。";
}

export function buildNextBestSourceRoute(input = {}) {
  const question = String(input.question || "").trim();
  const retrievalQuestion = String(input.retrievalQuestion || question).trim();
  const topic = compactGraphLabel(sanitizeGraphPromptText(retrievalQuestion || question || "本轮问题"), 90);
  const sources = Array.isArray(input.sources) ? input.sources : [];
  const claims = Array.isArray(input.evidenceChain?.claims) ? input.evidenceChain.claims : [];
  const sourceClaims = claims
    .filter((claim) => claim?.type === "source_evidence" && Number.isInteger(claim.sourceIndex))
    .filter((claim) => sources[claim.sourceIndex]);
  const sourceIndexes = uniqueOrdered(sourceClaims.map((claim) => claim.sourceIndex), 8);
  const authorCount = uniqueOrdered(sourceClaims.map((claim) => claim.author || sources[claim.sourceIndex]?.author || "").filter(Boolean), 8).length;
  const conflictCount = Number(input.knowledgeGapRadar?.metrics?.conflictCount || 0);
  const missingDataCount = Number(input.knowledgeGapRadar?.metrics?.missingDataCount || 0);
  const priorityGap = input.knowledgeGapRadar?.priority;
  const sourceCards = Array.isArray(input.sourceStudyPack?.sourceCards) ? input.sourceStudyPack.sourceCards : [];
  const topicSources = Array.isArray(input.topicSourceTree?.sources) ? input.topicSourceTree.sources : [];
  const criteria = [
    {
      id: "source-evidence",
      label: "先有作者原文",
      status: sourceClaims.length > 0 ? "ready" : "missing",
      detail: sourceClaims.length > 0
        ? `本轮已定位 ${sourceClaims.length} 条作者原文证据。`
        : "本轮还没有可定位作者原文，不能进入结论沉淀。",
    },
    {
      id: "independent-source",
      label: "再看独立来源",
      status: sourceIndexes.length >= 2 ? "ready" : "weak",
      detail: sourceIndexes.length >= 2
        ? `已覆盖 ${sourceIndexes.length} 条独立来源。`
        : "独立来源偏少，后续需要继续找支持或反驳材料。",
    },
    {
      id: "author-view",
      label: "再看作者视角",
      status: authorCount >= 2 ? "ready" : "weak",
      detail: authorCount >= 2
        ? `已覆盖 ${authorCount} 位作者视角。`
        : "作者视角偏少，可能只代表一种运营场景。",
    },
    {
      id: "conflict",
      label: "冲突先复核",
      status: conflictCount > 0 ? "needs_review" : "clear",
      detail: conflictCount > 0
        ? `发现 ${conflictCount} 个可能冲突点，先对照来源再行动。`
        : "暂未发现必须优先处理的来源冲突。",
    },
    {
      id: "business-data",
      label: "最后补业务数据",
      status: missingDataCount > 0 ? "needs_data" : "ready",
      detail: missingDataCount > 0
        ? `还缺 ${missingDataCount} 类你的产品或投放数据。`
        : "可以进入小范围验证或复盘。",
    },
  ];

  const sourceRoute = (claim, fallback = {}) => {
    const source = sources[claim?.sourceIndex] || {};
    const quote = compactGraphLabel(claim?.quote || claim?.text || source.excerpt || fallback.quote || "", 300);
    const title = source.title || claim?.title || fallback.title || `来源 ${Number(claim?.sourceIndex || 0) + 1}`;
    const author = source.author || claim?.author || fallback.author || "";
    return {
      id: `next-source:${claim?.id || claim?.sourceIndex || "primary"}`,
      kind: "source",
      label: compactGraphLabel(title, 120),
      author: compactGraphLabel(author, 80),
      title: compactGraphLabel(title, 180),
      sourceIndex: Number.isInteger(claim?.sourceIndex) ? claim.sourceIndex : fallback.sourceIndex,
      claimId: claim?.id || fallback.claimId || "",
      quote,
      reason: "这是本轮回答实际引用到的关键作者原文，先读它能最快判断回答是否站得住。",
      prompt: quote
        ? `请只基于这条作者原文解释它支撑了什么、没有支撑什么：${quote}`
        : `请只基于本轮第 ${Number(claim?.sourceIndex || 0) + 1} 条来源，解释它支撑了什么、没有支撑什么。`,
      canUseAsEvidence: false,
      sourceCanUseAsEvidence: true,
    };
  };

  let recommended;
  if (sourceClaims.length === 0) {
    recommended = {
      id: "next-source:search",
      kind: "source_search",
      label: "先补作者原文",
      reason: "没有可定位作者原文时，任何学习总结都只能当临时提示，不能沉淀成知识结论。",
      prompt: priorityGap?.prompt || `请重新检索「${topic}」的作者原文证据，并优先给出可引用片段。`,
      canUseAsEvidence: false,
      sourceCanUseAsEvidence: false,
    };
  } else {
    recommended = sourceRoute(sourceClaims[0]);
  }

  const alternatives = [];
  const addAlternative = (item) => {
    if (!item || !item.label || alternatives.some((existing) => existing.id === item.id || existing.label === item.label)) return;
    alternatives.push({ ...item, canUseAsEvidence: false });
  };

  for (const card of sourceCards) {
    if (!Number.isInteger(card.sourceIndex) || card.sourceIndex === recommended.sourceIndex) continue;
    addAlternative({
      id: `next-alt:source:${card.sourceIndex}`,
      kind: "source",
      label: compactGraphLabel(card.title || card.label || `来源 ${card.sourceIndex + 1}`, 120),
      author: compactGraphLabel(card.author, 80),
      title: compactGraphLabel(card.title, 180),
      sourceIndex: card.sourceIndex,
      claimId: card.claimId || "",
      quote: compactGraphLabel(card.quote, 260),
      reason: compactGraphLabel(card.why || "作为第二条来源核对，避免只凭单一片段行动。", 180),
      prompt: card.prompt || `请把来源 ${card.sourceIndex + 1} 拆成可核对的学习要点。`,
      sourceCanUseAsEvidence: true,
    });
    if (alternatives.length >= 2) break;
  }

  for (const treeSource of topicSources) {
    if (alternatives.length >= 3) break;
    if (!Number.isInteger(treeSource.sourceIndex) || treeSource.sourceIndex === recommended.sourceIndex) continue;
    addAlternative({
      id: `next-alt:tree-source:${treeSource.sourceIndex}`,
      kind: "source",
      label: compactGraphLabel(treeSource.title || treeSource.label || `来源 ${treeSource.sourceIndex + 1}`, 120),
      author: compactGraphLabel(treeSource.author, 80),
      title: compactGraphLabel(treeSource.title, 180),
      sourceIndex: treeSource.sourceIndex,
      claimId: treeSource.claimId || "",
      quote: compactGraphLabel(treeSource.quote, 260),
      reason: compactGraphLabel(treeSource.reason || "这条来源连接到本轮主题树，可用于交叉核对。", 180),
      prompt: treeSource.prompt || `请只看来源 ${treeSource.sourceIndex + 1}，说明它和本轮主题的关系。`,
      sourceCanUseAsEvidence: true,
    });
  }

  const radarGaps = Array.isArray(input.knowledgeGapRadar?.gaps) ? input.knowledgeGapRadar.gaps : [];
  for (const gap of radarGaps) {
    if (alternatives.length >= 4) break;
    if (!gap?.label || gap.id === "gap:source") continue;
    addAlternative({
      id: `next-alt:${gap.id || safeGraphId(gap.label)}`,
      kind: gap.kind || "gap",
      label: compactGraphLabel(gap.label, 100),
      reason: compactGraphLabel(gap.reason || "这是本轮继续学习前需要补的材料。", 180),
      prompt: gap.prompt || `请围绕「${topic}」继续补齐这个缺口。`,
      sourceIndexes: Array.isArray(gap.sourceIndexes) ? uniqueOrdered(gap.sourceIndexes.filter(Number.isInteger), 4) : [],
      claimIds: Array.isArray(gap.claimIds) ? uniqueOrdered(gap.claimIds.map((id) => String(id || "")).filter(Boolean), 4) : [],
      sourceCanUseAsEvidence: false,
    });
  }

  return {
    title: "下一步资料选择",
    status: sourceClaims.length === 0
      ? "needs_source"
      : conflictCount > 0
        ? "needs_review"
        : missingDataCount > 0
          ? "needs_data"
          : "ready_to_validate",
    summary: sourceClaims.length === 0
      ? "先重新找作者原文，不把临时回答沉淀成知识。"
      : `先读「${recommended.label}」，再按独立来源、作者视角和业务数据继续验证。`,
    topic,
    criteria,
    recommended,
    alternatives: alternatives.slice(0, 4),
    boundary: "下一步资料选择只安排阅读、复核和补材料顺序；推荐理由是系统整理，不是新的作者原文证据，也不会改变本地知识库内容。",
  };
}

export function buildTopicSourceTree(input = {}) {
  const question = String(input.question || "").trim();
  const retrievalQuestion = String(input.retrievalQuestion || question).trim();
  const answer = String(input.answer || "");
  const sources = Array.isArray(input.sources) ? input.sources : [];
  const claims = Array.isArray(input.evidenceChain?.claims) ? input.evidenceChain.claims : [];
  const sourceClaims = claims
    .filter((claim) => claim?.type === "source_evidence" && Number.isInteger(claim.sourceIndex))
    .filter((claim) => sources[claim.sourceIndex]);
  const topicLabel = compactGraphLabel(topicSourceTreeLabel(question, retrievalQuestion), 90);
  const topic = {
    id: "topic:question",
    label: topicLabel,
    question: compactGraphLabel(question || topicLabel, 160),
    retrievalQuestion: compactGraphLabel(retrievalQuestion || question || "", 220),
  };
  const conceptText = [
    retrievalQuestion,
    question,
    answer,
    ...sources.map((source) => `${source.title || ""}\n${source.excerpt || ""}`),
    ...sourceClaims.map((claim) => `${claim.text || ""}\n${claim.quote || ""}`),
  ].join("\n");
  const conceptLabels = detectGraphConcepts(conceptText);
  const concepts = (conceptLabels.length ? conceptLabels : ["待补来源"]).slice(0, 8).map((label, index) => ({
    id: `topic-concept:${safeGraphId(label || index)}`,
    label,
    identity: "系统整理",
    canUseAsEvidence: false,
    sourceIndexes: uniqueOrdered(
      sources
        .map((source, sourceIndex) => (textMentionsConcept(`${source.title || ""}\n${source.excerpt || ""}`, label) ? sourceIndex : null))
        .filter((sourceIndex) => Number.isInteger(sourceIndex)),
      4,
    ),
    prompt: `请围绕「${label}」解释本轮来源树，并区分作者原文和系统整理。`,
  }));

  if (sources.length === 0 || sourceClaims.length === 0) {
    return {
      title: "本轮主题来源树",
      status: "needs_source",
      topic,
      summary: "这轮还没有能定位到作者原文的来源树，只能先标出问题和待补来源。",
      boundary: "当前没有可绑定的作者原文，不能把系统整理、历史档案或用户业务材料当成来源证据。",
      concepts,
      sources: [],
      authors: [],
      paths: [
        {
          id: "topic-path:gap",
          kind: "gap",
          label: "先补作者原文",
          detail: `围绕「${topicLabel}」重新检索可引用来源，再展开主题来源树。`,
          prompt: `请重新检索「${topicLabel}」的作者原文证据，并说明缺少哪些来源。`,
        },
      ],
      nextPrompts: [
        `请重新检索「${topicLabel}」的作者原文证据。`,
        "请告诉我这轮缺少哪些来源，哪些判断不能采纳。",
      ],
    };
  }

  const claimBySource = new Map();
  for (const claim of sourceClaims) {
    if (!claimBySource.has(claim.sourceIndex)) claimBySource.set(claim.sourceIndex, claim);
  }
  const sourceRows = [...claimBySource.entries()]
    .slice(0, 5)
    .map(([sourceIndex, claim], index) => {
      const source = sources[sourceIndex] || {};
      const quote = compactGraphLabel(claim.quote || claim.text || source.excerpt || "", 260);
      const relatedConcepts = concepts
        .filter((concept) => textMentionsConcept(`${source.title || ""}\n${source.excerpt || ""}\n${quote}`, concept.label))
        .map((concept) => concept.label)
        .slice(0, 4);
      return {
        id: `topic-source:${index}`,
        claimId: claim.id || `source-evidence:${index}`,
        sourceIndex,
        identity: "作者原文",
        canUseAsEvidence: true,
        author: source.author || claim.author || "",
        date: source.date || claim.date || "",
        title: source.title || claim.title || `来源 ${sourceIndex + 1}`,
        sourceUrl: source.sourceUrl || "",
        sourcePath: source.sourcePath || "",
        label: compactGraphLabel(source.title || claim.text || `来源 ${sourceIndex + 1}`, 90),
        quote,
        reason: relatedConcepts.length
          ? `支撑本轮主题里的「${relatedConcepts.join("、")}」。`
          : "这是本轮回答实际引用到的作者原文。",
        concepts: relatedConcepts,
        prompt: `请只基于这条作者原文解释它支撑了什么、没有支撑什么：${quote}`,
      };
    });

  const authors = uniqueOrdered(sourceRows.map((item) => item.author).filter(Boolean), 4)
    .map((author, index) => {
      const rows = sourceRows.filter((source) => source.author === author);
      const authorConcepts = uniqueOrdered(rows.flatMap((source) => source.concepts || []), 5);
      return {
        id: `topic-author:${safeGraphId(author || index)}`,
        author,
        role: authorPerspectiveRole(author),
        sourceIndexes: rows.map((source) => source.sourceIndex),
        sourceCount: rows.length,
        concepts: authorConcepts,
        summary: `${authorPerspectiveRole(author)}：本轮可先读 ${rows.length} 条来源，重点核对${authorConcepts.length ? `「${authorConcepts.join("、")}」` : "问题适用条件"}。`,
      };
    });

  const paths = [
    {
      id: "topic-path:concept",
      kind: "concept",
      label: "问题先拆成概念",
      detail: concepts.map((item) => item.label).slice(0, 5).join("、") || "先拆清楚问题边界。",
      prompt: `请把「${topicLabel}」拆成亚马逊学习概念，并说明每个概念要查什么来源。`,
    },
    {
      id: "topic-path:source",
      kind: "source",
      label: "概念回到作者原文",
      detail: `本轮可定位 ${sourceRows.length} 条作者原文，先读引用片段再看系统整理。`,
      sourceIndex: sourceRows[0]?.sourceIndex,
      prompt: "请按来源顺序讲解本轮主题树，并标出哪些只是系统整理。",
    },
    ...authors.slice(0, 3).map((author, index) => ({
      id: `topic-path:author:${index}`,
      kind: "author",
      label: `${author.author} 的视角`,
      detail: author.summary,
      sourceIndex: author.sourceIndexes[0],
      prompt: `请只基于 ${author.author} 的本轮来源，解释这个问题的学习路径。`,
    })),
    {
      id: "topic-path:action",
      kind: "action",
      label: "转成下一步学习",
      detail: "把来源核对、作者视角和你的产品数据分开，不把系统整理当证据。",
      prompt: `请围绕「${topicLabel}」生成下一步学习顺序：先核对来源，再补产品数据，最后形成结论。`,
    },
  ];

  const followUpPrefix = retrievalQuestion && retrievalQuestion !== question ? "结合上一问主题，" : "";
  return {
    title: "本轮主题来源树",
    status: "ready",
    topic,
    summary: `这棵树把本轮问题连接到 ${concepts.length} 个概念、${sourceRows.length} 条作者原文和 ${authors.length} 位作者视角。`,
    boundary: "主题来源树只基于本轮可引用的作者原文、答案要点和来源元数据生成；概念、路径和下一步都是系统整理，不能当成新的作者证据。",
    concepts,
    sources: sourceRows,
    authors,
    paths,
    nextPrompts: [
      `${followUpPrefix}请按这棵主题来源树讲一遍「${topicLabel}」的学习顺序。`,
      "请帮我判断这轮应该先改哪一块，但必须标出来源和还缺的数据。",
      "请只看作者原文，列出本轮结论里哪些还不能下定论。",
    ],
  };
}

function topicSourceTreeLabel(question, retrievalQuestion) {
  const direct = String(question || "").replace(/\s+/g, " ").trim();
  const retrieval = String(retrievalQuestion || "").replace(/\s+/g, " ").trim();
  if (direct && (retrieval.length > 120 || /当前问题[:：]|上文背景[:：]|上轮已引用原文证据[:：]/.test(retrieval))) return direct;
  return retrieval || direct || "本轮问题";
}

export function buildAuthorPerspectiveRoom(input = {}) {
  const question = String(input.question || "").trim();
  const retrievalQuestion = String(input.retrievalQuestion || question).trim();
  const sources = Array.isArray(input.sources) ? input.sources : [];
  const claims = Array.isArray(input.evidenceChain?.claims) ? input.evidenceChain.claims : [];
  const sourceClaims = claims
    .filter((claim) => claim?.type === "source_evidence" && claim.id && Number.isInteger(claim.sourceIndex))
    .filter((claim) => sources[claim.sourceIndex]);
  const requestedCompare = isAuthorComparisonRequest(`${question}\n${retrievalQuestion}`);
  const conflictSignals = Array.isArray(input.evidenceAudit?.conflictSignals) ? input.evidenceAudit.conflictSignals : [];
  const hasConflict = conflictSignals.length > 0;
  const authorNames = uniqueOrdered(sourceClaims.map((claim) => sources[claim.sourceIndex]?.author).filter(Boolean), 3);

  if (sourceClaims.length === 0) {
    return {
      title: "跨作者观点对照",
      status: "needs_source",
      trigger: "missing_source",
      boundary: "当前没有可绑定的作者原文证据，不能生成跨作者观点；先补来源或重新提问。",
      authors: [],
      sharedConcepts: [],
      differences: [],
      requiredData: [],
      nextPrompt: `请围绕「${compactGraphLabel(retrievalQuestion || question || "这个问题", 80)}」重新检索作者原文，再做跨作者观点对照。`,
    };
  }

  if (!hasConflict && !requestedCompare) {
    return {
      title: "跨作者观点对照",
      status: "hidden",
      trigger: "no_conflict",
      boundary: "只在真实冲突或明确对比问题中显示；避免把互补观点硬凑成冲突。",
      authors: [],
      sharedConcepts: [],
      differences: [],
      requiredData: [],
      nextPrompt: "",
    };
  }

  const grouped = authorNames.map((author) => {
    const items = sourceClaims
      .filter((claim) => sources[claim.sourceIndex]?.author === author)
      .slice(0, 3)
      .map((claim, index) => {
        const source = sources[claim.sourceIndex] || {};
        const quote = compactGraphLabel(claim.quote || claim.text || source.excerpt || "", 280);
        return {
          id: `author-perspective:${safeGraphId(author)}:${index}`,
          claimId: claim.id,
          sourceIndex: claim.sourceIndex,
          identity: "作者原文",
          canUseAsEvidence: false,
          author,
          title: source.title || claim.title || `来源 ${claim.sourceIndex + 1}`,
          date: source.date || claim.date || "",
          sourceUrl: source.sourceUrl || "",
          sourcePath: source.sourcePath || "",
          quote,
          stance: authorPerspectiveStance(`${claim.text || ""}\n${quote}`),
          concepts: detectGraphConcepts(`${claim.text || ""}\n${quote}\n${source.title || ""}`).slice(0, 4),
        };
      });
    return {
      author,
      role: authorPerspectiveRole(author),
      items,
      summary: authorPerspectiveSummary(author, items),
    };
  }).filter((item) => item.items.length > 0);

  const topicText = [
    question,
    retrievalQuestion,
    ...grouped.flatMap((author) => author.items.map((item) => `${item.title}\n${item.quote}`)),
  ].join("\n");
  const sharedConcepts = detectGraphConcepts(topicText).slice(0, 6).map((label) => ({
    label,
    identity: "系统整理",
    canUseAsEvidence: false,
  }));
  const differences = buildAuthorPerspectiveDifferences(grouped, conflictSignals, sharedConcepts);
  const type = detectAnswerType(`${question}\n${retrievalQuestion}\n${topicText}`);
  const requiredData = buildValidationDataRequests(type, undefined, sources).slice(0, 5).map((item) => ({
    id: item.id,
    label: item.label,
    why: item.why,
    placeholder: item.placeholder,
  }));

  return {
    title: "跨作者观点对照",
    status: "ready",
    trigger: hasConflict ? "conflict" : "requested_compare",
    boundary: "这里只整理待核对的作者原文差异，不自动判断哪位作者正确；用户业务数据只用于判断适配性，不是作者原文证据。",
    authors: grouped,
    sharedConcepts,
    differences,
    requiredData,
    nextPrompt: buildAuthorPerspectiveNextPrompt(retrievalQuestion || question, requiredData, differences),
  };
}

function authorPerspectiveRole(author) {
  if (author === "张子卿") return "偏决策取舍";
  if (author === "飞翔的波波") return "偏执行和数据拆解";
  if (author === "跨境电商长期主义") return "偏系统沉淀和长期复用";
  return "作者原文视角";
}

function authorPerspectiveSummary(author, items = []) {
  const first = items[0];
  const concept = first?.concepts?.[0] || "本轮问题";
  const stance = first?.stance || "提醒先核对真实业务条件";
  return `${authorPerspectiveRole(author)}：围绕「${concept}」${stance}。`;
}

function authorPerspectiveStance(text) {
  const value = String(text || "");
  if (/不建议|不要|不能只|不是|先看|先检查|复核|承接|评价|价格|页面/.test(value)) return "提醒不要只看单一动作，先复核约束条件";
  if (/先|优先|必须|决定|入口|点击率|放大/.test(value)) return "强调优先处理当前入口或关键动作";
  return "提供补充视角，需结合产品数据判断适配性";
}

function buildAuthorPerspectiveDifferences(authors = [], conflictSignals = [], concepts = []) {
  const rows = [];
  const comparisons = conflictSignals
    .map((signal) => signal.comparison)
    .filter((comparison) => comparison && (comparison.supportQuote || comparison.cautionQuote));
  comparisons.slice(0, 3).forEach((comparison, index) => {
    rows.push({
      id: `difference:conflict:${index}`,
      label: compactGraphLabel(comparison.summary || "作者观点存在差异", 120),
      focus: compactGraphLabel(comparison.differenceFocus || comparison.suggestedCheck || "先补产品数据判断适配性。", 180),
      identity: "系统整理",
      canUseAsEvidence: false,
    });
  });
  if (rows.length === 0 && authors.length >= 2) {
    const labels = concepts.map((item) => item.label).filter(Boolean).slice(0, 3).join(" / ") || "本轮问题";
    rows.push({
      id: "difference:author-priority",
      label: `${labels} 的优先级和适用条件需要对照`,
      focus: "不同作者可能分别强调入口、承接、流量或长期能力；没有产品数据时不自动判胜负。",
      identity: "系统整理",
      canUseAsEvidence: false,
    });
  }
  return rows.slice(0, 3);
}

function buildAuthorPerspectiveNextPrompt(question, requiredData = [], differences = []) {
  const dataLabels = requiredData.map((item) => item.label).filter(Boolean).slice(0, 5);
  const diffText = differences.map((item) => item.label).filter(Boolean).slice(0, 2).join("；");
  return [
    `请基于本轮跨作者观点对照，判断「${compactGraphLabel(question || "这个问题", 100)}」下一步更适合采纳哪一侧。`,
    diffText ? `当前分歧：${diffText}` : "",
    dataLabels.length ? `我会补这些数据：${dataLabels.join("、")}。` : "请先告诉我还缺哪些产品数据。",
    "请明确哪些判断来自作者原文，哪些只是对我产品的适配性判断。",
  ].filter(Boolean).join("\n");
}

function buildStudyChecks(input = {}) {
  const type = input.type || "general";
  const takeaway = compactGraphLabel(input.takeaway || buildFallbackTakeaway(type), 180);
  const firstAction = compactGraphLabel(input.nextActions?.[0] || buildExecutionSteps(input.retrievalQuestion || input.question)[0], 180);
  const firstMissing = compactGraphLabel(input.missingInputs?.[0] || "补具体产品、页面或广告数据", 120);
  const firstSource = Array.isArray(input.sources) ? input.sources[0] : null;
  const sourceLabel = firstSource
    ? compactGraphLabel(`${firstSource.author || "未知作者"}《${firstSource.title || "未命名来源"}》`, 180)
    : "";
  const sourceQuote = firstSource?.excerpt ? compactGraphLabel(firstSource.excerpt, 180) : "";
  const checks = [
    {
      id: "takeaway",
      kind: "takeaway",
      question: "这轮回答最核心的判断是什么？",
      expectedAnswer: takeaway,
      sourceIndex: firstSource ? 0 : undefined,
      prompt: `我还没完全理解这轮核心判断：${takeaway}。请用作者来源重新解释一遍，并说明哪些只是系统整理。`,
    },
    {
      id: "source",
      kind: "source",
      question: "哪条作者来源最能支撑这个判断？",
      expectedAnswer: sourceQuote ? `${sourceLabel}：${sourceQuote}` : sourceLabel,
      sourceIndex: 0,
      prompt: `请只围绕 ${sourceLabel} 这条来源，解释它支撑了什么、没有支撑什么。`,
    },
    {
      id: "action",
      kind: "action",
      question: "如果要把这轮回答变成动作，第一步应该做什么？",
      expectedAnswer: firstAction,
      sourceIndex: firstSource ? 0 : undefined,
      prompt: `我准备执行这一步：${firstAction}。请帮我拆成低风险检查清单，并标出需要补的数据。`,
    },
    {
      id: "boundary",
      kind: "boundary",
      question: "这轮哪些内容不能当成作者原文证据？",
      expectedAnswer: `用户产品数据、实验结果和系统整理的行动建议都不能替代作者原文；下一步至少要补：${firstMissing}。`,
      prompt: "请帮我区分这轮回答里的作者原文、系统推断、用户业务材料和下一步验证动作。",
    },
  ];

  return checks
    .filter((item) => item.question && item.expectedAnswer)
    .slice(0, 4)
    .map((item) => ({
      id: compactGraphLabel(item.id, 60),
      kind: compactGraphLabel(item.kind, 40),
      question: compactGraphLabel(item.question, 180),
      expectedAnswer: compactGraphLabel(item.expectedAnswer, 320),
      prompt: compactGraphLabel(item.prompt, 360),
      sourceIndex: Number.isInteger(item.sourceIndex) ? item.sourceIndex : undefined,
      boundary: "理解检查只用于学习复盘，不产生新证据，也不会写入原始知识库。",
    }));
}

export function buildWorkflowIntent(input = {}) {
  const question = String(input.question || "");
  const retrievalQuestion = String(input.retrievalQuestion || question);
  const text = `${retrievalQuestion}\n${question}`;
  const preferredType = normalizeWorkflowIntentPreference(input.intentPreference);
  const productInputSummary = input.productInputSummary;
  const hasProductFacts = Array.isArray(productInputSummary?.facts) && productInputSummary.facts.length > 0;
  const hasSources = Array.isArray(input.sources) && input.sources.length > 0;
  const followUp = Array.isArray(input.learningCard?.followUps) ? input.learningCard.followUps[0] || "" : "";
  const validationPrompt = input.validationPack?.followUpPrompt || "";
  const hasExperimentEvidence =
    /验证数据|实验名称|回填/.test(text) ||
    /(CTR|CVR|ACOS|点击率|转化率)[^\n]{0,24}前\s*[\/／和后]|前\s*[\/／和后][^\n]{0,24}(CTR|CVR|ACOS|点击率|转化率)/i.test(text) ||
    /数据[^\n]{0,24}前\s*[\/／和后]|前\s*[\/／和后][^\n]{0,24}数据|前后数据|数据前后/.test(text) ||
    /结论[:：]/.test(text);
  const hasProductDiagnosisCue =
    /我的|我这个|这个产品|这个\s*Listing|当前产品|当前\s*Listing|我们这个|该先改哪|先改哪|值不值得做|是否值得做|卖不动|不出单|没点击|没转化|核心关键词[:：]|ASIN[:：]?/i.test(text) ||
    /(CTR|CVR|ACOS|点击率|转化率|CPC|ROAS)\s*[:：=是为约大概]?\s*\d/i.test(text);
  const hasExplicitRetryCue = /重查|再查|重新检索|换一种问法|引用位置不准|引用不准|来源不对|证据不对|重新回答/.test(text);

  if (preferredType) {
    return workflowIntentTemplate(preferredType, {
      followUp,
      validationPrompt,
      hasProductFacts,
      hasSources,
      confidence: "user_confirmed",
    });
  }

  if (hasExperimentEvidence) {
    return workflowIntentTemplate("experiment_review", { validationPrompt, confidence: "high" });
  }

  if (hasProductFacts || hasProductDiagnosisCue) {
    return workflowIntentTemplate("product_diagnosis", { validationPrompt, hasProductFacts, confidence: hasProductFacts ? "high" : "medium" });
  }

  if (hasExplicitRetryCue || /重新判断/.test(text)) {
    return workflowIntentTemplate("answer_retry", { confidence: "medium" });
  }

  if (hasSources) {
    return workflowIntentTemplate("method_learning", { followUp, confidence: "high" });
  }

  return workflowIntentTemplate("source_search", { confidence: "medium" });
}

export function workflowIntentTemplate(type, input = {}) {
  const normalizedType = normalizeWorkflowIntentPreference(type) || "method_learning";
  const confidence = input.confidence || "medium";
  if (normalizedType === "experiment_review") {
    return {
      type: "experiment_review",
      label: "实验复盘",
      goal: "把你的实验前后数据和本轮作者证据分开对照。",
      primaryAction: "回看实验前后数据，再判断继续放大、补测，还是回到页面基础。",
      nextPrompt: input.validationPrompt || "我补充了实验结果，请结合作者来源判断下一步。",
      boundary: "实验结果只验证你的业务，不改写作者原文，也不能替代作者原文证据。",
      confidence,
    };
  }
  if (normalizedType === "product_diagnosis") {
    return {
      type: "product_diagnosis",
      label: "产品诊断",
      goal: "用你的产品事实对照作者方法，找下一步优先级。",
      primaryAction: "先看诊断优先级，再补缺失材料或进入小实验验证。",
      nextPrompt: input.validationPrompt || "请结合我补充的产品数据和本地资料，判断下一步优先级。",
      boundary: "用户产品材料不是作者原文证据，只用于判断这些方法是否适配你的产品。",
      confidence,
    };
  }
  if (normalizedType === "answer_retry") {
    return {
      type: "answer_retry",
      label: "重查回答",
      goal: "重新组织检索，不沿用被你标记有问题的旧回答证据。",
      primaryAction: "换更具体的问题、排除不相关来源，或限定作者后重新问。",
      nextPrompt: "请重新检索这个问题，并优先给出可核对的作者原文来源。",
      boundary: "被标记重查或引用不准的旧回答不会作为下一轮证据依据。",
      confidence,
    };
  }
  if (normalizedType === "source_search") {
    return {
      type: "source_search",
      label: "补来源检索",
      goal: "先找到足够相关的作者原文，再进入学习或诊断。",
      primaryAction: "换更具体的问题，加入选品、关键词、主图、广告或 Listing 等主题词。",
      nextPrompt: "请换一种更具体的问法重新检索作者资料。",
      boundary: "没有作者原文时，回答只能作为检索建议，不能沉淀成知识结论。",
      confidence,
    };
  }
  return {
    type: "method_learning",
    label: "方法学习",
    goal: "先理解作者方法，并定位能支撑判断的原文证据。",
    primaryAction: "先核对关键来源，再决定采纳、排除，或带产品数据继续问。",
    nextPrompt: input.followUp || "请基于这些作者来源，继续拆成可执行检查清单。",
    boundary: "这一步只把作者原文作为资料证据；你的业务数据需要单独补充验证。",
    confidence,
  };
}

function normalizeWorkflowIntentPreference(value) {
  const raw = typeof value === "object" && value ? value.type : value;
  const type = String(raw || "").trim();
  return ["method_learning", "product_diagnosis", "experiment_review", "answer_retry", "source_search"].includes(type)
    ? type
    : "";
}

export function buildLearningQueue(input = {}) {
  const sources = Array.isArray(input.sources) ? input.sources : [];
  const sourceClaims = Array.isArray(input.evidenceChain?.claims)
    ? input.evidenceChain.claims.filter((claim) => claim?.type === "source_evidence" && Number.isInteger(claim.sourceIndex))
    : [];
  const conflictSignals = Array.isArray(input.evidenceAudit?.conflictSignals) ? input.evidenceAudit.conflictSignals : [];
  const studyChecks = Array.isArray(input.learningCard?.studyChecks) ? input.learningCard.studyChecks : [];
  const dataRequests = Array.isArray(input.validationPack?.dataRequests) ? input.validationPack.dataRequests : [];
  const experiments = Array.isArray(input.validationPack?.experiments) ? input.validationPack.experiments : [];
  const workflowIntent = input.workflowIntent || {};
  const items = [];
  const hasEvidenceGate = sourceClaims.length > 0;
  const addItem = (item) => {
    if (!item || !item.id || items.some((existing) => existing.id === item.id)) return;
    items.push({
      id: compactGraphLabel(item.id, 80),
      kind: compactGraphLabel(item.kind || "task", 40),
      label: compactGraphLabel(item.label || "", 120),
      reason: compactGraphLabel(item.reason || "", 180),
      action: compactGraphLabel(item.action || "", 40),
      actionLabel: compactGraphLabel(item.actionLabel || "继续", 60),
      completionMode: compactGraphLabel(item.completionMode || "manual", 40),
      requiresEvidenceGate: item.requiresEvidenceGate === true,
      lockedLabel: typeof item.lockedLabel === "string" ? compactGraphLabel(item.lockedLabel, 80) : undefined,
      boundary: compactGraphLabel(item.boundary || "队列只记录学习动作是否处理过，不代表结论已经被证明。", 220),
      sourceIndex: Number.isInteger(item.sourceIndex) ? item.sourceIndex : undefined,
      claimId: typeof item.claimId === "string" ? compactGraphLabel(item.claimId, 80) : undefined,
      prompt: typeof item.prompt === "string" ? compactGraphLabel(item.prompt, 420) : undefined,
      section: typeof item.section === "string" ? compactGraphLabel(item.section, 40) : undefined,
      done: item.done === true,
    });
  };

  const firstClaim = sourceClaims[0];
  if (firstClaim) {
    addItem({
      id: "queue:evidence",
      kind: "evidence",
      label: "核对关键原文证据",
      reason: firstClaim.quote
        ? `先确认这条原文是否真的支撑本轮判断：${compactGraphLabel(firstClaim.quote, 120)}`
        : firstClaim.text || "先确认本轮回答最关键的作者原文证据。",
      action: "review-evidence",
      actionLabel: "定位证据",
      sourceIndex: firstClaim.sourceIndex,
      claimId: firstClaim.id,
      completionMode: "evidence_feedback",
      boundary: "核对原文只确认证据存在，不等于你的产品已经适用这个结论。",
    });
  } else {
    addItem({
      id: "queue:source-search",
      kind: "source_search",
      label: "先补更具体的来源检索",
      reason: sources.length > 0
        ? "这轮虽然有候选来源，但没有形成可采纳的原文证据，先换成更具体的问题重新查资料。"
        : "这轮没有稳定的作者原文支撑，先换成更具体的问题重新查资料。",
      action: "fill-prompt",
      actionLabel: "填入追问",
      prompt: workflowIntent.nextPrompt || "请换一种更具体的问法重新检索作者资料。",
      completionMode: "manual",
      boundary: "没有作者原文时，队列只能帮助继续检索，不能沉淀成知识结论。",
    });
  }

  if (conflictSignals.length > 0) {
    addItem({
      id: "queue:conflict",
      kind: "conflict",
      label: "再对比冲突来源",
      reason: `发现 ${conflictSignals.length} 个可能冲突点，先确认关键原文，再打开双方来源决定是否采纳结论。`,
      action: "review-audit",
      actionLabel: "看冲突点",
      section: "audit",
      completionMode: "manual",
      requiresEvidenceGate: true,
      lockedLabel: "先核对证据",
      boundary: "冲突检查只是风险提示，不会判断哪位作者一定正确。",
    });
  }

  const firstStudyCheck = studyChecks[0];
  if (firstStudyCheck) {
    addItem({
      id: `queue:study:${firstStudyCheck.id || firstStudyCheck.kind || "check"}`,
      kind: "study_check",
      label: "做一次理解检查",
      reason: firstStudyCheck.question || "用一个小问题确认自己真的理解了本轮核心判断。",
      action: "fill-prompt",
      actionLabel: "填入理解追问",
      prompt: firstStudyCheck.prompt,
      sourceIndex: firstStudyCheck.sourceIndex,
      completionMode: "manual",
      requiresEvidenceGate: true,
      lockedLabel: hasEvidenceGate ? "先核对证据" : "先补来源",
      boundary: "理解检查只用于学习复盘，不产生新证据，也不会写入原始知识库。",
    });
  }

  const firstDataRequest = dataRequests[0];
  if (firstDataRequest) {
    addItem({
      id: `queue:data:${firstDataRequest.id || "request"}`,
      kind: "data",
      label: "补齐真实业务材料",
      reason: firstDataRequest.why || firstDataRequest.label || "把产品、页面、关键词或广告数据补进来，再判断方法是否适用。",
      action: "fill-data",
      actionLabel: "去补数据",
      completionMode: "manual",
      requiresEvidenceGate: true,
      lockedLabel: hasEvidenceGate ? "先核对证据" : "先补来源",
      boundary: "用户业务材料只用于验证你的业务，不会变成作者原文证据。",
    });
  }

  const firstExperiment = experiments[0];
  if (firstExperiment) {
    addItem({
      id: `queue:experiment:${firstExperiment.id || "validation"}`,
      kind: "experiment",
      label: "设计一轮低风险验证",
      reason: firstExperiment.title || "把本轮判断转成能回填结果的小实验。",
      action: "review-validation",
      actionLabel: "看验证任务",
      section: "validation",
      completionMode: "manual",
      requiresEvidenceGate: true,
      lockedLabel: hasEvidenceGate ? "先核对证据" : "先补来源",
      boundary: "验证任务只是实验计划，只有回填真实结果后才算业务复盘。",
    });
  }

  if (input.learningCard && sources.length > 0) {
    addItem({
      id: "queue:dossier",
      kind: "dossier",
      label: "保存为待复核档案",
      reason: "把本轮回答、待核对来源和下一步保存到本地学习档案，后续再确认哪些证据可采纳。",
      action: "save-dossier",
      actionLabel: "保存档案",
      completionMode: "manual",
      requiresEvidenceGate: true,
      lockedLabel: hasEvidenceGate ? "先核对证据" : "先补来源",
      boundary: "保存档案不代表确认结论，也不会自动采纳来源；业务材料和实验结果仍会和作者原文分开。",
    });
  }

  if (workflowIntent.nextPrompt) {
    addItem({
      id: "queue:follow-up",
      kind: "follow_up",
      label: "带着上一步继续追问",
      reason: workflowIntent.primaryAction || "沿着本轮意图继续拆下一步。",
      action: "fill-prompt",
      actionLabel: "填入追问",
      prompt: workflowIntent.nextPrompt,
      completionMode: "manual",
      requiresEvidenceGate: true,
      lockedLabel: hasEvidenceGate ? "先核对证据" : "先补来源",
      boundary: "继续追问会重新检索资料；旧回答不会自动变成新证据。",
    });
  }

  const limitedItems = items.slice(0, 6).map((item) => {
    if (item.id === "queue:evidence" || item.id === "queue:source-search") return item;
    if (item.requiresEvidenceGate !== true) return item;
    return {
      ...item,
      requiresEvidenceGate: true,
      lockedLabel: item.lockedLabel || (hasEvidenceGate ? "先核对证据" : "先补来源"),
      locked: item.done !== true,
      lockedReason: hasEvidenceGate
        ? "先把关键原文证据标记为“有用”或“不相关”，再进入这一步。"
        : "先补到可采纳的作者原文证据，再进入这一步。",
    };
  });
  const completed = limitedItems.filter((item) => item.done).length;
  const total = limitedItems.length;
  const currentItem = limitedItems.find((item) => !item.done) || limitedItems[0];
  return {
    summary: total > 0
      ? "把本轮回答推进成证据核对、理解检查、业务验证和学习沉淀的连续动作。"
      : "这轮回答暂时没有形成可推进的学习动作。",
    boundary: "学习队列只记录你处理了哪些学习动作，不代表结论正确，也不会改写作者原文。",
    currentItemId: currentItem?.id || "",
    progress: {
      completed,
      total,
      percent: total > 0 ? Math.round((completed / total) * 100) : 0,
    },
    items: limitedItems,
  };
}

export function buildValidationPack(
  question,
  answer,
  sources = [],
  retrievalQuestion = question,
  evidenceChain = undefined,
  productInputSummary = undefined,
  productDiagnosis = undefined,
) {
  const type = detectAnswerType(`${retrievalQuestion}\n${question}`);
  const sourceClaims = Array.isArray(evidenceChain?.claims)
    ? evidenceChain.claims.filter((claim) => claim?.type === "source_evidence" && Number.isInteger(claim.sourceIndex))
    : [];
  const status = sourceClaims.length > 0 ? "source_backed" : "needs_source";
  const dataRequests = buildValidationDataRequests(type, productInputSummary, sources);
  const experiments = buildValidationExperiments(type, status);
  const decisionRules = buildValidationDecisionRules(type, status);
  const sourceActionConstraint = buildSourceActionConstraint(`${retrievalQuestion}\n${question}`, sourceClaims);
  const businessDecision = buildBusinessDecision(type, productInputSummary, {
    status,
    dataRequests,
    productDiagnosis,
    sourceActionConstraint,
  });
  const hypotheses = sourceClaims.slice(0, 3).map((claim, index) => {
    const source = sources[claim.sourceIndex] || {};
    return {
      id: `hypothesis:${index}`,
      label: compactGraphLabel(claim.quote || claim.text || claim.title || "本轮来源证据", 120),
      sourceIndex: claim.sourceIndex,
      author: claim.author || source.author || "",
      sourceTitle: claim.title || source.title || "",
      quote: compactGraphLabel(claim.quote || claim.text || "", 220),
      verifyWith: buildValidationPrompt(type),
    };
  });

  return {
    title: "本轮业务验证任务包",
    status,
    summary: status === "source_backed"
      ? "把本轮作者证据转成真实业务数据检查，避免只停留在资料总结。"
      : "这轮缺少作者原文证据，只能先列验证方向，不能直接沉淀成结论。",
    boundary: status === "source_backed"
      ? "任务包里的假设来自本轮作者原文证据；用户回填的数据只用于复核，不会变成作者原文证据。"
      : "这轮没有作者原文证据；用户数据和学习档案只能提出验证方向，不能替代作者原文证据。",
    hypotheses,
    dataRequests,
    experiments,
    decisionRules,
    businessDecision,
    followUpPrompt: buildValidationFollowUpPrompt(type, dataRequests, status),
  };
}

function buildBusinessDecision(type, productInputSummary, options = {}) {
  const status = options.status === "source_backed" ? "source_backed" : "needs_source";
  const facts = productInputSummary && Array.isArray(productInputSummary.facts)
    ? flattenProductFacts(productInputSummary)
    : [];
  const boundary = status === "source_backed"
    ? "当前产品判断使用作者原文证据 + 用户产品数据；用户产品数据不是作者原文证据，也不会自动改写学习档案。"
    : "当前缺少可采纳作者原文；用户产品数据不能替代作者原文证据，不要直接下最终判断。";
  const base = {
    title: "当前产品判断",
    status: "needs_data",
    priority: "insufficient",
    label: "数据不足，暂时不能判断先改哪一块。",
    summary: "先补关键产品数据，再判断作者观点更适合哪种情况。",
    supportingData: [],
    opposingData: [],
    missingData: [],
    boundary,
  };
  const sourceActionConstraint = options.sourceActionConstraint || {};

  if (status !== "source_backed") {
    return {
      ...base,
      status: "needs_source",
      missingData: [{ id: "source", label: "作者原文证据", why: "先找到可定位来源，再用产品数据复核。" }],
    };
  }

  if (facts.length === 0) {
    return {
      ...base,
      status: "needs_data",
      missingData: businessDecisionMissingItems(type, null, null, productInputSummary, options.dataRequests),
    };
  }

  if (sourceActionConstraint.status === "caution") {
    return {
      ...base,
      status: "needs_review",
      priority: "source_caution",
      label: "先核对来源反向提醒，暂不能直接判定先改主图。",
      summary: "本轮作者来源明确不建议先改主图；即使用户数据里 CTR 偏低，也要先核对评价、价格、页面承接和同屏竞品条件，再决定主图是否进入验证任务。",
      supportingData: [],
      opposingData: [],
      missingData: businessDecisionMissingItems(type, null, null, productInputSummary, options.dataRequests),
      boundary,
    };
  }

  if (sourceActionConstraint.status === "conflict") {
    return {
      ...base,
      status: "needs_review",
      priority: "source_conflict",
      label: "来源对主图优先级有分歧，先核对后再判断。",
      summary: "本轮来源同时出现支持和反向信号，不能把主图作为固定第一步；先补齐 CTR、CVR、评价、价格和页面承接数据，再决定采纳哪一侧。",
      supportingData: [],
      opposingData: [],
      missingData: businessDecisionMissingItems(type, null, null, productInputSummary, options.dataRequests),
      boundary,
    };
  }

  const visual = factsForLabels(productInputSummary, /主图|视觉/);
  const metrics = uniqueRawItems([...factsForLabels(productInputSummary, /点击率|转化率|数据/), ...metricLikeFacts(facts)]);
  const listing = factsForLabels(productInputSummary, /Listing|页面/);
  const ads = uniqueRawItems([...factsForLabels(productInputSummary, /广告|流量/), ...adLikeFacts(facts)]);
  const keywords = factsForLabels(productInputSummary, /关键词|竞品/);
  const ctr = extractPercentMetric(metrics, /\bctr\b|点击率/i);
  const cvr = extractPercentMetric(metrics, /\bcvr\b|转化率/i);
  const acos = extractPercentMetric(ads.length > 0 ? ads : facts, /\bacos\b/i);
  const hasPriceReviewRisk = /价格.{0,12}(高|贵|高于|没有优势)|评价.{0,12}(少|低|差)|评分.{0,12}(低|差|[1-3]\.)/i.test(facts.join("\n"));
  const supportingData = [
    metricDecisionData("ctr", "CTR/点击率", ctr),
    metricDecisionData("cvr", "CVR/转化率", cvr),
    metricDecisionData("acos", "ACOS", acos),
    visual.length ? { id: "visual", label: "主图/视觉", value: firstFact(visual), role: "supporting" } : null,
    listing.length ? { id: "listing", label: "Listing/页面", value: firstFact(listing), role: "supporting" } : null,
    keywords.length ? { id: "keywords", label: "关键词/竞品", value: firstFact(keywords), role: "supporting" } : null,
  ].filter(Boolean);
  const missingData = businessDecisionMissingItems(type, ctr, cvr, productInputSummary, options.dataRequests);

  if (type === "visual" && (ctr === null || cvr === null)) {
    return {
      ...base,
      status: "insufficient_data",
      supportingData,
      missingData,
      summary: "现在还不能区分点击入口问题和页面承接问题。",
    };
  }

  if (hasPriceReviewRisk) {
    return {
      ...base,
      status: "ready",
      priority: "price_review",
      label: "当前先查价格、评价和页面信任，再决定是否改主图。",
      summary: "你提供的数据里出现价格或评价阻力；即使主图能带来点击，也可能被页面信任问题抵消。",
      supportingData,
      opposingData: ctr !== null && ctr < 0.4 ? [{ id: "ctr", label: "CTR 偏低", value: `${ctr}%`, role: "opposing" }] : [],
      missingData,
      boundary,
    };
  }

  if (type === "visual" && ctr !== null && ctr < 0.4 && (cvr === null || cvr >= 4)) {
    return {
      ...base,
      status: "ready",
      priority: "main_image",
      label: "当前优先改主图点击入口。",
      summary: "CTR 偏低而 CVR 没有同步崩掉，说明问题更像发生在搜索结果入口；先改主图和同屏差异化，再看页面细节。",
      supportingData,
      missingData,
      boundary,
    };
  }

  if (type === "visual" && ctr !== null && ctr >= 0.4 && cvr !== null && cvr < 4) {
    return {
      ...base,
      status: "ready",
      priority: "listing_bridge",
      label: "当前先修 Listing 页面承接。",
      summary: "CTR 不算低但 CVR 偏低，更像点击进来之后没有被页面、价格、评价或副图说服。",
      supportingData,
      missingData,
      boundary,
    };
  }

  if (acos !== null && acos >= 40 && cvr !== null && cvr < 5) {
    return {
      ...base,
      status: "ready",
      priority: "listing_bridge",
      label: "当前先修页面承接，再调广告。",
      summary: "ACOS 偏高且 CVR 偏低，广告更可能是在放大页面基本面问题。",
      supportingData,
      missingData,
      boundary,
    };
  }

  return {
    ...base,
    status: missingData.length > 0 ? "insufficient_data" : "ready",
    priority: missingData.length > 0 ? "insufficient" : "balanced_check",
    label: missingData.length > 0 ? "数据不足，不能直接判定单一优先级。" : "当前先做入口、页面和流量的并行核对。",
    summary: options.productDiagnosis?.reason || "现有数据没有形成单一强信号，先按作者证据拆开验证。",
    supportingData,
    missingData,
    boundary,
  };
}

function businessDecisionMissingItems(type, ctr, cvr, productInputSummary, dataRequests = []) {
  const rows = [];
  const add = (id, label, why) => {
    if (rows.some((item) => item.id === id || item.label === label)) return;
    rows.push({ id, label, why });
  };
  if (type === "visual") {
    if (ctr === null) add("ctr", "当前点击率 CTR", "没有 CTR 就不能判断是否先发生在搜索结果点击入口。");
    if (cvr === null) add("cvr", "当前转化率 CVR", "没有 CVR 就不能判断点击后的页面承接是否掉链。");
    const facts = productInputSummary && Array.isArray(productInputSummary.facts) ? flattenProductFacts(productInputSummary) : [];
    if (!/竞品|前三|搜索结果|同屏|asin/i.test(facts.join("\n"))) {
      add("competitor", "核心词同屏竞品", "需要确认主图、价格、评价比较的是同一组真实货架。");
    }
  }
  if (rows.length === 0 && !productInputSummary && Array.isArray(dataRequests)) {
    dataRequests.slice(0, 3).forEach((item) => add(item.id || item.label, item.label, item.why));
  }
  return rows.slice(0, 5);
}

function metricDecisionData(id, label, value) {
  if (value === null || value === undefined) return null;
  return { id, label, value: `${value}%`, role: "supporting" };
}

function buildValidationDataRequests(type, productInputSummary, sources = []) {
  const requests = [];
  const add = (id, label, why, placeholder = "") => {
    if (requests.some((item) => item.id === id || item.label === label)) return;
    requests.push({
      id,
      label: compactGraphLabel(label, 90),
      why: compactGraphLabel(why, 160),
      placeholder: compactGraphLabel(placeholder, 120),
    });
  };

  if (type === "visual") {
    add("ctr", "当前点击率 CTR", "判断问题是否先发生在搜索结果点击入口。", "例如：核心词 CTR 0.25%，曝光 12000，点击 30");
    add("cvr", "当前转化率 CVR", "区分点击入口问题和页面承接问题。", "例如：CVR 5.1%，Session 1200，订单 61");
    add("image", "主图/副图截图", "确认主图差异化和副图解释力。", "例如：主图白底，副图缺少尺寸对比");
    add("competitor", "核心关键词下前三个竞品", "确认你比较的是同一组真实货架。", "例如：关键词 garlic press，竞品 ASIN...");
  } else if (type === "product") {
    add("demand", "核心关键词搜索量和头部销量", "验证市场容量是否足够。", "例如：月搜索量、前三名月销量");
    add("margin", "售价、成本和推广预算", "验证利润能否承受冷启动。", "例如：售价、FBA、毛利、CPC");
    add("risk", "评分、退货和差评风险", "避免只看需求而忽略硬风险。", "例如：主要差评点、退货原因");
  } else if (type === "listing") {
    add("keyword-map", "关键词词库和用户任务", "确认标题、五点、Search Terms 的承接分工。", "例如：核心词、长尾词、场景词");
    add("listing-copy", "当前标题、五点和 A+", "检查页面是否解释差异和信任理由。", "例如：标题前 80 字、五点、A+ 模块");
    add("search-terms", "收录和广告搜索词报告", "用真实搜索词反向修正页面。", "例如：已收录词、出单词、浪费词");
  } else if (type === "ads") {
    add("ad-metrics", "广告花费、CPC、ACOS、CVR", "区分预算问题、流量问题和页面问题。", "例如：SP ACOS 45%，CPC 1.2，CVR 4%");
    add("search-report", "搜索词报告", "判断哪些词该加预算、否词或暂停。", "例如：高花费无订单词、出单词");
    add("listing-base", "Listing 基本面", "广告只能放大基本面，不能替代页面承接。", "例如：主图、价格、评价、标题");
  } else {
    add("goal", "本轮业务目标", "先明确是在解决选品、页面、流量还是复盘。", "例如：提升 CTR、降低 ACOS、判断是否继续做");
    add("product", "具体产品、关键词和页面状态", "让下一轮回答能落到真实场景。", "例如：类目、核心词、价格、评价");
  }

  const missing = Array.isArray(productInputSummary?.missing) ? productInputSummary.missing : [];
  missing.slice(0, 3).forEach((item, index) => add(`missing:${index}`, item, "这是本轮诊断还缺的用户业务材料。"));
  if (sources.length === 0) {
    add("source-scope", "更具体的作者资料问题", "先让知识库命中作者原文，再把业务数据放进去复核。", "例如：围绕主图点击率、Listing 承接或广告浪费追问");
  }
  return requests.slice(0, 6);
}

function buildValidationExperiments(type, status) {
  if (status === "needs_source") {
    return [
      {
        id: "source-first",
        title: "先补作者证据再做判断",
        steps: ["换一个更具体的问题", "确认命中作者原文来源", "再补产品数据做复核"],
        successSignal: "下一轮回答出现可定位的作者来源和证据链。",
      },
    ];
  }
  if (type === "visual") {
    return [
      {
        id: "visual-split",
        title: "主图点击入口小实验",
        steps: ["保留价格和广告不变", "只替换主图差异化表达", "观察 7 天 CTR、CVR 和 ACOS"],
        successSignal: "CTR 明显上升且 CVR 不下降，说明主图入口优先级成立。",
      },
    ];
  }
  if (type === "ads") {
    return [
      {
        id: "ad-waste",
        title: "广告浪费切分实验",
        steps: ["标记高花费无订单词", "对浪费词降价或否词", "把预算集中到有效词小跑 7 天"],
        successSignal: "ACOS 下降且订单不明显减少，说明流量结构需要重排。",
      },
    ];
  }
  if (type === "listing") {
    return [
      {
        id: "listing-bridge",
        title: "Listing 承接验证",
        steps: ["保持主图和广告稳定", "改标题前段和五点承接", "对比核心词 CTR、CVR 和出单词变化"],
        successSignal: "CVR 或相关出单词改善，说明页面承接是主要瓶颈。",
      },
    ];
  }
  return [
    {
      id: "decision-gate",
      title: "小范围验证决策门",
      steps: ["先补关键数据", "只改一个变量", "用 7 天结果决定继续、修改或停止"],
      successSignal: "能用数据解释是否继续投入，而不是只凭感觉推进。",
    },
  ];
}

function buildValidationDecisionRules(type, status) {
  if (status === "needs_source") {
    return [
      { if: "没有作者原文来源", then: "只把本轮结果当成待验证方向，不保存为已采纳结论。" },
      { if: "下一轮命中作者来源", then: "再把业务数据和作者证据分开复核。" },
    ];
  }
  if (type === "visual") {
    return [
      { if: "CTR 上升但转化率 CVR 不动", then: "主图入口可能改善了，下一步转向副图、五点、评价和价格承接。" },
      { if: "CTR 不动且曝光稳定", then: "主图差异化或搜索结果货架对比仍是优先问题。" },
      { if: "CTR 上升但 ACOS 变差", then: "检查点击是否变宽泛，广告词和页面承接需要一起复核。" },
    ];
  }
  if (type === "ads") {
    return [
      { if: "ACOS 下降但订单减少", then: "预算压得过狠，需要重新区分探索词和转化词。" },
      { if: "高花费无订单词持续出现", then: "先否词或降价，再看 Listing 是否承接错流量。" },
    ];
  }
  if (type === "listing") {
    return [
      { if: "收录增加但转化不动", then: "关键词覆盖改善了，但卖点、图片或价格承接仍要复核。" },
      { if: "出单词和页面核心词不一致", then: "重排标题、五点和 Search Terms 的关键词分工。" },
    ];
  }
  return [
    { if: "关键数据支持本轮判断", then: "保存为学习档案并进入实验复盘。" },
    { if: "关键数据反驳本轮判断", then: "回到作者来源和竞品货架重新拆问题。" },
  ];
}

function buildValidationFollowUpPrompt(type, dataRequests, status) {
  const labels = dataRequests.map((item) => item.label).slice(0, 4).join("、");
  const base = status === "source_backed"
    ? "我补充了验证数据，请结合本轮作者来源和这些数据判断下一步："
    : "我补充了验证数据，但这轮还缺作者原文证据，请先帮我重新找来源再判断：";
  const typeHint = describeIntent(type).label;
  return `${base}\n主题：${typeHint}\n需要核对：${labels}`;
}

function extractAnswerSections(answer) {
  const conclusions = [];
  const steps = [];
  let section = "";

  for (const rawLine of String(answer || "").split("\n")) {
    const line = rawLine.trim();
    if (line === "可执行结论：") {
      section = "conclusions";
      continue;
    }
    if (line === "执行顺序：") {
      section = "steps";
      continue;
    }
    if (/^(资料里最相关的判断|作者视角|建议下一步)：?/.test(line)) {
      section = "";
      continue;
    }
    const item = line.match(/^\d+\.\s+(.+)$/)?.[1]?.trim();
    if (!item) continue;
    if (section === "conclusions") conclusions.push(stripInlineEvidenceMarkers(item));
    if (section === "steps") steps.push(stripInlineEvidenceMarkers(item));
  }

  return { conclusions, steps };
}

function stripInlineEvidenceMarkers(text) {
  return String(text || "")
    .replace(/\s*【(?:资料|证据|来源|推断)\d+】/g, "")
    .replace(/\s*【行动\d+】/g, "")
    .replace(/\s*【缺少来源】/g, "")
    .trim();
}

function detectGraphConcepts(text) {
  return GRAPH_CONCEPTS.filter((concept) => concept.pattern.test(text)).map((concept) => concept.label);
}

function textMentionsConcept(text, conceptLabel) {
  const concept = GRAPH_CONCEPTS.find((item) => item.label === conceptLabel);
  return concept ? concept.pattern.test(String(text || "")) : String(text || "").includes(conceptLabel);
}

function compactGraphLabel(value, maxLength) {
  const text = String(value || "")
    .replace(/\s+/g, " ")
    .trim();
  if (text.length <= maxLength) return text;
  return `${text.slice(0, Math.max(1, maxLength - 1))}…`;
}

function safeGraphId(value) {
  return String(value || "item")
    .replace(/[^a-zA-Z0-9:_-]+/g, "-")
    .slice(0, 80);
}

function normalizeProductInput(productInput) {
  if (!productInput || typeof productInput !== "object") return { hasValue: false, text: "", intake: null };
  const wrappedIntake = productInput.intake && typeof productInput.intake === "object" ? productInput.intake : null;
  const directIntake = Array.isArray(productInput.sections) ? productInput : null;
  const text = String(productInput.text || productInput.rawText || "").slice(0, 3000);
  const intakeCandidate = wrappedIntake || directIntake;
  const candidateHasSections = Array.isArray(intakeCandidate?.sections) && intakeCandidate.sections.length > 0;
  const intake = candidateHasSections ? intakeCandidate : text ? buildProductIntake({ text }) : intakeCandidate;
  return {
    hasValue: !!(intake || text),
    text,
    intake,
  };
}

function uniqueSafeList(items, limit, maxLength) {
  if (!Array.isArray(items)) return [];
  const seen = new Set();
  const rows = [];
  for (const item of items) {
    const text = compactGraphLabel(item, maxLength);
    if (!text || seen.has(text)) continue;
    seen.add(text);
    rows.push(text);
    if (rows.length >= limit) break;
  }
  return rows;
}

function selectSourceArticles(question, articles, ranked, limit, options = {}) {
  const selected = [];
  const seen = new Set();
  const addArticle = (article) => {
    if (!article) return false;
    const key = sourceKey(article);
    if (seen.has(key)) return false;
    seen.add(key);
    selected.push(article);
    return true;
  };

  if (options.diversifyAuthors && articles.length > 1 && limit > 1) {
    const profile = buildQuestionProfile(question);
    const articleAuthors = uniqueOrdered(articles.map((article) => article.author).filter(Boolean), limit);
    const orderedAuthors = [
      ...AUTHORS.filter((author) => articleAuthors.includes(author)),
      ...articleAuthors.filter((author) => !AUTHORS.includes(author)),
    ].slice(0, limit);

    for (const author of orderedAuthors) {
      const rankedCandidate = ranked.find((point) => point?.source?.author === author && !seen.has(sourceKey(point.source)));
      if (addArticle(rankedCandidate?.source)) continue;
      const scoredCandidate = articles.find(
        (article) =>
          article.author === author &&
          !seen.has(sourceKey(article)) &&
          bestEvidenceSentenceForArticle(article, profile),
      );
      addArticle(scoredCandidate);
      if (selected.length >= limit) return selected;
    }
  }

  for (const point of ranked) {
    addArticle(point.source);
    if (selected.length >= limit) return selected;
  }

  return selected;
}

function bestEvidenceSentenceForArticle(article, profile) {
  return splitSentences(article?.body || article?.excerpt || "")
    .map((text) => ({ text, score: scoreText(text, profile) }))
    .filter((item) => item.score > 0)
    .sort((a, b) => b.score - a.score || b.text.length - a.text.length)[0];
}

function sourceKey(source) {
  const baseKey = legacySourceKey(source);
  const sourcePath = String(source?.sourcePath || "").trim();
  const sourceUrl = String(source?.sourceUrl || "").trim();
  const stableIdentity = [sourcePath, sourceUrl].filter(Boolean).join("|");
  if (stableIdentity) return `${baseKey}|${stableIdentity}`;
  return `${baseKey}|${sourceContentFingerprint(source)}`;
}

function legacySourceKey(source) {
  const author = String(source?.author || "").trim();
  const date = String(source?.date || "").trim();
  const title = String(source?.title || "").trim();
  return `${author}|${date}|${title}`;
}

function sourceContentFingerprint(source) {
  const text = String(source?.body || source?.excerpt || "")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 800);
  if (!text) return "";
  let hash = 5381;
  for (let index = 0; index < text.length; index += 1) {
    hash = ((hash << 5) + hash + text.charCodeAt(index)) >>> 0;
  }
  return hash.toString(36);
}

function sourceIdentityKeys(source) {
  return [source.sourcePath, source.sourceUrl, sourceKey(source), legacySourceKey(source)]
    .filter((key) => typeof key === "string" && key.trim())
    .map((key) => key.trim());
}

function isSourceExcluded(source, excludedSourceKeys) {
  if (!(excludedSourceKeys instanceof Set) || excludedSourceKeys.size === 0) return false;
  return sourceIdentityKeys(source).some((key) => excludedSourceKeys.has(key));
}

function isSourceAllowed(source, allowedAuthors, allowedSourceKeys = new Set()) {
  const hasAuthorScope = allowedAuthors instanceof Set && allowedAuthors.size > 0;
  const hasSourceScope = allowedSourceKeys instanceof Set && allowedSourceKeys.size > 0;
  if (!hasAuthorScope && !hasSourceScope) return true;
  if (hasAuthorScope && !allowedAuthors.has(String(source?.author || "").trim())) return false;
  if (hasSourceScope && !sourceIdentityKeys(source).some((key) => allowedSourceKeys.has(key))) return false;
  return true;
}

function normalizeSourceKeySet(keys) {
  if (!Array.isArray(keys)) return new Set();
  return new Set(keys.filter((key) => typeof key === "string" && key.trim()).map((key) => key.trim()).slice(0, 100));
}

function normalizeAuthorSet(authors) {
  if (!Array.isArray(authors)) return new Set();
  return new Set(authors.map((author) => String(author || "").trim()).filter(Boolean).slice(0, 20));
}

function buildSourceScopeSummary(rawArticles, scopedArticles, allowedAuthors, allowedSourceKeys = new Set(), options = {}) {
  const activeAuthors = [...(allowedAuthors instanceof Set ? allowedAuthors : new Set())];
  const activeSourceKeys = [...(allowedSourceKeys instanceof Set ? allowedSourceKeys : new Set())];
  const activeSourceCount = Number.isFinite(Number(options.allowedSourceCount)) && Number(options.allowedSourceCount) > 0
    ? Math.floor(Number(options.allowedSourceCount))
    : activeSourceKeys.length;
  const active = activeAuthors.length > 0 || activeSourceKeys.length > 0;
  return {
    active,
    allowedAuthors: activeAuthors,
    allowedSourceKeys: activeSourceKeys,
    allowedSourceCount: activeSourceCount,
    totalRetrieved: Array.isArray(rawArticles) ? rawArticles.length : 0,
    totalAfterScope: Array.isArray(scopedArticles) ? scopedArticles.length : 0,
    summary: activeSourceKeys.length > 0
      ? `本轮只使用已选择的 ${activeSourceCount} 个来源。`
      : activeAuthors.length > 0
        ? `本轮只使用 ${activeAuthors.join("、")} 的资料。`
        : "本轮使用全部作者资料。",
    caution: active
      ? "研究范围只影响本次问答可用来源，不会删除或改写原始知识库。"
      : "你可以在左侧按作者锁定研究范围。",
  };
}

export function buildRetrievalQuery(question, history = [], options = {}) {
  const current = retrievalCurrentQuestionText(question);
  const safeHistory = Array.isArray(history) ? history : [];
  const recent = safeHistory.slice(-6);
  if (safeHistory.length === 0) return current;
  const excludedSourceKeys = normalizeSourceKeySet(options.excludedSourceKeys);

  const memoryAnchors = safeHistory.slice(0, Math.max(0, safeHistory.length - recent.length))
    .map((entry) => retrievalMemoryAnchorLine(entry, excludedSourceKeys))
    .filter(Boolean)
    .slice(-4);
  const recentLines = recent
    .map((entry) => retrievalHistoryLine(entry, excludedSourceKeys))
    .filter(Boolean);
  const contextLines = dedupeRetrievalLines([...memoryAnchors, ...recentLines]);
  const lines = [`当前问题：${current}`];
  if (contextLines.length > 0) {
    lines.push("", "上文背景：", ...contextLines);
  }

  return compactHistoryText(lines.join("\n"), 1700);
}

function retrievalMemoryAnchorLine(entry, excludedSourceKeys = new Set()) {
  if (!entry || entry.role !== "assistant") return "";
  const effectiveness = answerEffectivenessRetrievalSignal(entry);
  const feedback = entry.evidenceFeedback && typeof entry.evidenceFeedback === "object" ? entry.evidenceFeedback : {};
  const hasUsefulEvidence = Object.values(feedback).some((value) => value === "useful");
  if (!effectiveness.status && !hasUsefulEvidence) return "";
  return retrievalHistoryLine(entry, excludedSourceKeys);
}

function dedupeRetrievalLines(lines = []) {
  const seen = new Set();
  const result = [];
  for (const line of lines) {
    const key = String(line || "").trim();
    if (!key || seen.has(key)) continue;
    seen.add(key);
    result.push(key);
  }
  return result;
}

function retrievalCurrentQuestionText(question) {
  const raw = String(question || "").trim();
  const redacted = compactRetrievalUserText(raw);
  if (redacted) return redacted;
  return compactHistoryText(redactBusinessFactsForRetrieval(raw), 500) || raw.slice(0, 120);
}

function retrievalHistoryLine(entry, excludedSourceKeys = new Set()) {
  if (!entry || typeof entry !== "object") return "";
  if (entry.role === "user") {
    return `用户问题：${compactRetrievalUserText(entry.content || entry.answer || "")}`;
  }
  if (entry.role !== "assistant") return "";

  const effectiveness = answerEffectivenessRetrievalSignal(entry);
  if (["needs_source", "switch_intent", "add_product_data"].includes(effectiveness.status)) {
    return effectiveness.line;
  }
  const evidence = sourceEvidenceClaims(entry, excludedSourceKeys);
  if (evidence.claims.length > 0) {
    const evidenceLine = `${evidence.label}：${evidence.claims
      .map((claim) => {
        const quote = compactHistoryText(claim.quote || claim.text || "", 180);
        const title = claim.title ? `（${claim.title}）` : "";
        return `${quote}${title}`;
      })
      .join(" / ")}`;
    return [effectiveness.line, evidenceLine].filter(Boolean).join("\n");
  }
  if (evidence.hadSourceEvidence) return "";
  if (entry.restoredFromDossierId) return "";
  if (effectiveness.line) return effectiveness.line;
  return "";
}

function answerEffectivenessRetrievalSignal(entry) {
  const effectiveness = entry?.answerEffectiveness && typeof entry.answerEffectiveness === "object" ? entry.answerEffectiveness : {};
  const status = normalizeAnswerEffectivenessStatus(effectiveness.status);
  if (!status) return { status: "", line: "" };
  const question = compactRetrievalUserText(effectiveness.question || "");
  const suffix = question ? `原问题：${question}` : "";
  if (status === "needs_source") {
    return {
      status,
      line: compactHistoryText(`用户确认上轮需要补来源：优先重新检索作者原文证据。${suffix}`),
    };
  }
  if (status === "switch_intent") {
    return {
      status,
      line: compactHistoryText(`用户确认上轮需要切换意图：下一轮先判断是方法学习、产品诊断、实验复盘还是补来源检索。${suffix}`),
    };
  }
  if (status === "add_product_data") {
    const topicSuffix = answerEffectivenessTopicSuffix(effectiveness.question || "");
    return {
      status,
      line: compactHistoryText(`用户确认上轮需要补产品数据：检索只保留通用主题，产品细节要和作者原文证据分开处理。${topicSuffix}`),
    };
  }
  return {
    status,
    line: compactHistoryText(`用户确认上轮回答有效：延续这个学习主题。${suffix}`),
  };
}

function normalizeAnswerEffectivenessStatus(value) {
  return ["resolved", "needs_source", "switch_intent", "add_product_data"].includes(value) ? value : "";
}

function answerEffectivenessTopicSuffix(question) {
  const concepts = detectGraphConcepts(String(question || "")).slice(0, 6);
  if (concepts.length > 0) return `主题：${concepts.join("、")}`;
  const redacted = compactRetrievalUserText(question);
  return redacted ? `主题：${redacted}` : "";
}

function compactRetrievalUserText(value) {
  const lines = String(value || "")
    .split(/\n+/)
    .map((line) => cleanRetrievalUserLine(line))
    .filter(Boolean);
  return compactHistoryText(lines.join(" "));
}

function cleanRetrievalUserLine(line) {
  const normalized = String(line || "")
    .replace(/^[-*]\s+/, "")
    .replace(/^\d+[.)、]\s*/, "")
    .trim();
  if (!normalized) return "";
  const questionLike = /[？?]/.test(normalized);
  if (!questionLike && isBusinessFactLine(normalized)) return "";
  return redactBusinessFactsForRetrieval(normalized);
}

function isBusinessFactLine(value) {
  const text = String(value || "");
  return (
    hasBusinessMetricValue(text) ||
    /(?:核心关键词|竞品|对标|产品\/ASIN|ASIN|供应商报价|采购价|出厂价|拿货价)\s*[:：]?/i.test(text) ||
    /(?:主图|图片).*(?:白底|同质|差不多|竞品|现状)/.test(text) ||
    /(?:我的|我这个|我们这个).*(?:产品|Listing|listing)/.test(text)
  );
}

function hasBusinessMetricValue(value) {
  const text = String(value || "");
  return (
    /\b(?:CTR|CVR|ACOS|CPC|CPA|ROAS)\b\s*(?:[:：=]|[-–—]|is|was|是|为|约为|大约|约)?\s*[-+]?\d/i.test(text) ||
    /\b(?:click[-\s]*through rate|conversion rate|advertising cost of sales)\b\s*(?:[:：=]|[-–—]|is|was|是|为|约为|大约|约)?\s*[-+]?\d/i.test(text) ||
    /(?:点击率|转化率|预算|售价|销量|评分|评价数|session|Sessions)\s*(?:[:：=]|是|为|约为|大约|约)?\s*[-+]?\d/i.test(text)
  );
}

function redactBusinessFactsForRetrieval(value) {
  return redactNaturalBusinessEntities(String(value || ""))
    .replace(/\b(CTR|CVR|ACOS|CPC|CPA|ROAS)\b\s*(?:[:：=]|[-–—]|is|was|是|为|约为|大约|约)?\s*[-+]?\d+(?:\.\d+)?\s*%?/gi, "$1")
    .replace(/\b(click[-\s]*through rate|conversion rate|advertising cost of sales)\b\s*(?:[:：=]|[-–—]|is|was|是|为|约为|大约|约)?\s*[-+]?\d+(?:\.\d+)?\s*%?/gi, "$1")
    .replace(/(点击率|转化率|预算|售价|销量|评分|评价数|session|Sessions)\s*(?:[:：=]|是|为|约为|大约|约)?\s*[-+]?\d+(?:\.\d+)?\s*%?/gi, "$1")
    .replace(/(?:核心关键词|关键词|搜索词|核心词|keywords?|search terms?)\s*[:：=]?\s*[^，。；;、\n]+/gi, (match) => {
      const label = match.match(/^(核心关键词|关键词|搜索词|核心词|keywords?|search terms?)/i)?.[1] || "关键词";
      return label;
    })
    .replace(/竞品\s*(?:ASIN)?\s*[:：]?\s*\b[A-Z0-9]{8,}\b/gi, "竞品")
    .replace(/\bB0[A-Z0-9]{8}\b/gi, "ASIN")
    .replace(/\s+/g, " ")
    .trim();
}

function redactNaturalBusinessEntities(value) {
  return String(value || "")
    .replace(/(供应商报价|供应报价|采购价|出厂价|拿货价|报价|成本)\s*(?:[:：=]|是|为|约为|大约|约)?\s*[$￥¥]?\s*[-+]?\d+(?:\.\d+)?\s*(?:美金|美元|USD|usd|元|RMB|rmb)?/gi, "$1")
    .replace(/(竞品|对标竞品|对标|供应商)\s*(?:[:：=]|是|为)?\s*([A-Za-z][A-Za-z0-9&'’.-]*(?:\s+[A-Za-z][A-Za-z0-9&'’.-]*){0,4})(?=[，,。；;、\n]|$)/gi, (_match, label) => {
      if (/对标/i.test(label)) return "对标竞品";
      return label;
    })
    .replace(/((?:我这个|我的|我们这个|我们这款|这个|这款)\s*)([A-Za-z][A-Za-z0-9&'’.-]*(?:\s+[A-Za-z][A-Za-z0-9&'’.-]*){0,5})(?=[，,。；;、？?\n]|$)/gi, (_match, prefix, entity) => {
      const normalized = String(entity || "").trim().toLowerCase();
      if (isGenericBusinessEntityLabel(normalized)) return `${prefix}${entity.trim()}`;
      return `${prefix.trim()}产品`;
    });
}

function isGenericBusinessEntityLabel(value) {
  return [
    "listing",
    "product",
    "asin",
    "sku",
    "ctr",
    "cvr",
    "acos",
    "cpc",
    "roas",
    "review",
    "reviews",
    "keyword",
    "keywords",
  ].includes(String(value || "").trim().toLowerCase());
}

function sourceEvidenceClaims(entry, excludedSourceKeys = new Set()) {
  const claims = Array.isArray(entry?.evidenceChain?.claims) ? entry.evidenceChain.claims : [];
  const feedback = entry?.evidenceFeedback && typeof entry.evidenceFeedback === "object" ? entry.evidenceFeedback : {};
  const auditFeedback = normalizeAuditFeedbackValue(entry?.evidenceAudit?.feedback);
  if (auditFeedback === "retry" || auditFeedback === "citation_wrong") {
    return {
      label: "上轮证据已被用户要求重查",
      claims: [],
      hadSourceEvidence: claims.some((claim) => claim?.type === "source_evidence"),
    };
  }
  const sources = Array.isArray(entry?.sources) ? entry.sources : [];
  let sourceOrdinal = 0;
  const sourceClaims = claims
    .filter((claim) => claim?.type === "source_evidence" && (claim.quote || claim.text))
    .filter((claim) => !isClaimSourceExcluded(claim, sources, excludedSourceKeys))
    .map((claim) => {
      const fallbackId = `source-evidence:${sourceOrdinal}`;
      sourceOrdinal += 1;
      return {
        ...claim,
        id: claim.id || fallbackId,
        feedback: normalizeEvidenceFeedbackValue(feedback?.[claim.id || fallbackId]),
      };
    });
  const useful = sourceClaims.filter((claim) => claim.feedback === "useful");
  if (useful.length > 0) {
    return {
      label: "用户标记有用的原文",
      claims: useful.slice(0, 3),
      hadSourceEvidence: true,
    };
  }
  return {
    label: "上轮已引用原文证据",
    claims: sourceClaims.filter((claim) => claim.feedback !== "irrelevant").slice(0, 3),
    hadSourceEvidence: sourceClaims.length > 0,
  };
}

function isClaimSourceExcluded(claim, sources, excludedSourceKeys) {
  if (!(excludedSourceKeys instanceof Set) || excludedSourceKeys.size === 0) return false;
  const source = Number.isInteger(claim?.sourceIndex) ? sources[claim.sourceIndex] || {} : {};
  const merged = {
    ...source,
    author: claim?.author || source.author,
    date: claim?.date || source.date,
    title: claim?.title || source.title,
    sourcePath: claim?.sourcePath || source.sourcePath,
    sourceUrl: claim?.sourceUrl || source.sourceUrl,
  };
  return sourceIdentityKeys(merged).some((key) => excludedSourceKeys.has(key));
}

function normalizeEvidenceFeedbackValue(value) {
  return value === "useful" || value === "irrelevant" ? value : "";
}

function normalizeAuditFeedbackValue(value) {
  return value === "useful" || value === "citation_wrong" || value === "retry" ? value : "";
}

export function normalizeContextText(value) {
  if (typeof value === "string") return value;
  if (value && typeof value === "object") {
    const chunks = value?.data?.context?.chunks || value?.context?.chunks || value?.chunks;
    if (Array.isArray(chunks)) {
      return chunks
        .map((chunk) => articleChunkToText(chunk))
        .filter(Boolean)
        .join("\n\n");
    }
    if (typeof value?.data?.llm_context_message === "string") return value.data.llm_context_message;
    if (typeof value.result === "string") return value.result;
    if (typeof value.context === "string") return value.context;
    if (typeof value.text === "string") return value.text;
  }
  return value == null ? "" : String(value);
}

export function normalizeLearningMemoryReminder(value) {
  const chunks = contextChunksFrom(value)
    .filter(isLearningMemoryChunk)
    .map((chunk, index) => learningMemoryChunkToItem(chunk, index))
    .filter(Boolean);
  const seen = new Set();
  const items = [];
  for (const item of chunks) {
    const key = `${item.namespace}|${item.key}|${item.documentId}|${item.title}`;
    if (seen.has(key)) continue;
    seen.add(key);
    items.push(item);
    if (items.length >= 3) break;
  }

  return {
    label: "本地学习档案提醒",
    boundary: "这些是你保存过的学习档案提醒，不是作者原文证据；历史业务材料和实验复盘只用于验证你的业务，不能替代作者原文证据；本轮引用和证据链仍只来自作者资料。",
    items,
  };
}

function withLearningMemoryAlignment(reminder, sources = [], rankedEvidence = []) {
  const items = Array.isArray(reminder?.items) ? reminder.items : [];
  if (items.length === 0) return reminder;
  const alignment = buildLearningMemoryAlignment(items, sources, rankedEvidence);
  return {
    ...reminder,
    alignment,
  };
}

function buildLearningMemoryAlignment(items = [], sources = [], rankedEvidence = []) {
  const evidence = Array.isArray(rankedEvidence) ? rankedEvidence.filter((item) => Number.isInteger(item.sourceIndex)) : [];
  if (!Array.isArray(sources) || sources.length === 0 || evidence.length === 0) {
    return {
      status: "needs_source",
      label: "缺少本轮作者证据",
      message: "本轮没有足够作者原文证据，学习档案只能提醒过去思路，不能替代作者原文证据。",
      conflicts: [],
      matches: [],
    };
  }

  const memoryRows = learningMemoryStanceRows(items);
  const evidenceRows = evidenceStanceRows(evidence, sources);
  const conflicts = [];
  const matches = [];

  for (const memory of memoryRows) {
    for (const source of evidenceRows) {
      if (conflictFamilyForConcept(memory.concept) !== conflictFamilyForConcept(source.concept)) continue;
      if (memory.stance && source.stance && memory.stance !== source.stance) {
        conflicts.push({
          concept: memory.concept,
          memoryTitle: memory.title,
          memoryExcerpt: memory.excerpt,
          memoryStance: memory.stance,
          sourceStance: source.stance,
          sourceIndex: source.sourceIndex,
          sourceTitle: source.title,
          author: source.author,
          quote: source.quote,
        });
      } else if (memory.stance && source.stance && memory.stance === source.stance) {
        matches.push({
          concept: memory.concept,
          memoryTitle: memory.title,
          sourceIndex: source.sourceIndex,
          sourceTitle: source.title,
          author: source.author,
        });
      }
    }
  }

  const safeConflicts = dedupeLearningAlignmentRows(conflicts).slice(0, 3);
  if (safeConflicts.length > 0) {
    return {
      status: "conflict",
      label: "历史档案与本轮作者证据不一致",
      message: "历史档案与本轮作者证据不一致，先以本轮作者证据为准，再用你的产品数据复核。",
      conflicts: safeConflicts,
      matches: [],
    };
  }

  const safeMatches = dedupeLearningAlignmentRows(matches).slice(0, 3);
  if (safeMatches.length > 0) {
    return {
      status: "aligned",
      label: "历史档案与本轮证据方向一致",
      message: "本轮作者证据与历史档案方向一致，但学习档案仍只是提醒，不是作者原文证据。",
      conflicts: [],
      matches: safeMatches,
    };
  }

  return {
    status: "neutral",
    label: "历史档案仅作提醒",
    message: "本轮作者证据与历史档案没有形成明确一致或冲突关系，先按作者原文和你的业务数据判断。",
    conflicts: [],
    matches: [],
  };
}

function learningMemoryStanceRows(items = []) {
  const rows = [];
  items.forEach((item) => {
    if (["business_material", "experiment_review"].includes(item?.memoryKind)) return;
    const text = `${item.title || ""}\n${item.excerpt || ""}`;
    conflictConceptsForText(text).forEach((concept) => {
      const stance = learningMemoryStanceForText(text);
      if (!stance) return;
      rows.push({
        concept,
        stance,
        title: item.title || "学习档案",
        excerpt: item.excerpt || "",
      });
    });
  });
  return rows;
}

function learningMemoryStanceForText(text) {
  const value = String(text || "").replace(/\s+/g, "");
  const supportPattern = /必须先|必须优先|第一优先级|先把|先看|先改|先优化|优先.*(改|做|优化|检查|处理)|核心瓶颈|决定.*点击率|入口处理/;
  const cautionPattern = /不建议先|不用先|先别急着|优先级不高|不是当前瓶颈|不是.*(关键|重点|优先|核心|瓶颈)|不要.*(先|优先|只|仅)|不能.*(先|优先|只|靠)|暂缓|低优先级/;
  const hasSupport = supportPattern.test(value);
  const hasCaution = cautionPattern.test(value);
  if (hasSupport && !hasCaution) return "support";
  if (hasCaution && !hasSupport) return "caution";
  return "";
}

function evidenceStanceRows(evidence = [], sources = []) {
  const rows = [];
  evidence.forEach((item) => {
    const text = `${item.quote || ""}\n${item.title || ""}`;
    conflictConceptsForText(text).forEach((concept) => {
      const stance = conflictStanceForText(text, concept);
      if (!stance) return;
      const source = Number.isInteger(item.sourceIndex) ? sources[item.sourceIndex] || {} : {};
      rows.push({
        concept,
        stance,
        sourceIndex: item.sourceIndex,
        title: item.title || source.title || "",
        author: item.author || source.author || "",
        quote: compactGraphLabel(item.quote || "", 140),
      });
    });
  });
  return rows;
}

function dedupeLearningAlignmentRows(rows = []) {
  const seen = new Set();
  const result = [];
  for (const row of rows) {
    const key = `${row.concept}|${row.memoryTitle || ""}|${row.sourceIndex ?? ""}|${row.sourceTitle || ""}`;
    if (seen.has(key)) continue;
    seen.add(key);
    result.push(row);
  }
  return result;
}

function contextChunksFrom(value) {
  if (!value) return [];
  if (Array.isArray(value)) return value;
  if (typeof value !== "object") return [];
  const candidates = [
    value?.data?.context?.chunks,
    value?.context?.chunks,
    value?.result?.data?.context?.chunks,
    value?.result?.context?.chunks,
    value?.chunks,
  ];
  return candidates.find((candidate) => Array.isArray(candidate)) || [];
}

function isLearningMemoryChunk(chunk) {
  if (!chunk || typeof chunk !== "object") return false;
  const metadata = chunk.metadata || {};
  const namespace = String(metadata.namespace || chunk.namespace || "").trim();
  const sourceType = String(metadata.source_type || metadata.sourceType || chunk.source_type || "").trim();
  const key = String(metadata.key || chunk.key || "").trim();
  const title = String(metadata.title || chunk.title || "").trim();
  const content = String(chunk.content || chunk.text || "").trim();
  return (
    /-workflow$/.test(namespace) ||
    sourceType === "amazon-learning-dossier" ||
    key.startsWith("dossier/") ||
    /亚马逊学习档案|学习档案/.test(`${title}\n${content}`)
  );
}

function learningMemoryChunkToItem(chunk, index) {
  const metadata = chunk.metadata || {};
  const content = String(chunk.content || chunk.text || "").trim();
  if (!content) return null;
  const scoreBreakdown = metadata.score_breakdown || metadata.scoreBreakdown || {};
  const title = metadata.title || chunk.title || markdownTitle(content) || "学习档案";
  const namespace = metadata.namespace || chunk.namespace || "";
  const key = metadata.key || chunk.key || "";
  const excerptInfo = learningMemoryExcerptInfo(content);
  return {
    id: compactGraphLabel(`memory:${key || chunk.document_id || index}`, 120),
    title: compactGraphLabel(title, 80),
    excerpt: excerptInfo.excerpt,
    memoryKind: excerptInfo.memoryKind,
    memoryKindLabel: excerptInfo.memoryKindLabel,
    namespace: compactGraphLabel(namespace, 80),
    key: compactGraphLabel(key, 160),
    documentId: compactGraphLabel(chunk.document_id || chunk.documentId || metadata.document_id || "", 120),
    score: boundedScore(chunk.score ?? metadata.score ?? scoreBreakdown.final_score),
    vectorSimilarity: boundedScore(scoreBreakdown.vector_similarity ?? scoreBreakdown.vectorSimilarity),
    sourceType: compactGraphLabel(metadata.source_type || metadata.sourceType || "", 80),
  };
}

function markdownTitle(content) {
  const match = String(content || "").match(/^#\s+(.+)$/m);
  return match ? match[1].trim() : "";
}

function learningMemoryExcerptInfo(content) {
  const memoryKind = learningMemoryKind(content);
  return {
    excerpt: learningMemoryExcerpt(content, memoryKind),
    memoryKind,
    memoryKindLabel: learningMemoryKindLabel(memoryKind),
  };
}

function learningMemoryKind(content) {
  const value = String(content || "");
  if (/实验复盘|实验结果|小实验|CTR\s*从|CVR\s*从|ACOS\s*从/.test(value)) return "experiment_review";
  if (/已采纳原文证据|作者原文证据/.test(value)) return "source_evidence_note";
  if (
    /用户业务材料|业务验证记录|产品材料|产品\/ASIN|主图现状|核心关键词|竞品\/对标/.test(value) ||
    hasBusinessMetricValue(value) ||
    isBusinessFactLine(value)
  ) {
    return "business_material";
  }
  return "study_note";
}

function learningMemoryKindLabel(memoryKind) {
  if (memoryKind === "business_material") return "历史业务材料";
  if (memoryKind === "experiment_review") return "历史实验复盘";
  if (memoryKind === "source_evidence_note") return "历史证据笔记";
  return "历史学习笔记";
}

function learningMemoryExcerpt(content, memoryKind = "study_note") {
  const cleaned = String(content || "")
    .split(/\n+/)
    .map((line) =>
      line
        .replace(/^#+\s*/, "")
        .replace(/^[-*]\s+/, "")
        .replace(/^\d+[.)、]\s*/, "")
        .trim(),
    )
    .filter((line) => line && !/^(问题|当前结论|下一步|待验证|来源边界|保存时间|用户业务材料|业务验证记录|产品材料|实验复盘|实验结果)$/.test(line))
    .filter((line) => !/^亚马逊学习档案[:：]/.test(line))
    .filter((line) => !/[？?]$/.test(line));
  const preferredPattern = learningMemoryExcerptPattern(memoryKind);
  const preferred =
    cleaned.find((line) => preferredPattern.test(line) && line.length >= 8) ||
    cleaned.find((line) => line.length >= 8) ||
    "";
  return compactGraphLabel(preferred, 220);
}

function learningMemoryExcerptPattern(memoryKind) {
  if (memoryKind === "business_material") {
    return /主图|CTR|CVR|ACOS|核心关键词|竞品|Listing|广告|产品|ASIN|点击率|转化率/;
  }
  if (memoryKind === "experiment_review") {
    return /实验|复盘|结果|CTR\s*从|CVR\s*从|ACOS\s*从|前\/后|前后|改动|结论/;
  }
  return /主图|点击率|转化率|Listing|广告|关键词|先/;
}

function boundedScore(value) {
  const number = Number(value);
  if (!Number.isFinite(number)) return undefined;
  return Math.max(0, Math.min(1, Math.round(number * 1000) / 1000));
}

function articleChunkToText(chunk) {
  if (typeof chunk === "string") return chunk;
  if (!chunk || typeof chunk !== "object") return "";
  if (isLearningMemoryChunk(chunk)) return "";

  const content = String(chunk.content || chunk.text || "").trim();
  const metadata = chunk.metadata || {};
  if (!content) return "";

  const needsHeader = !/^#\s+/m.test(content);
  if (!needsHeader) return content;

  const title = metadata.title || metadata.key || "未命名资料";
  const author = metadata.category || metadata.author || "";
  const date = String(metadata.date || metadata.published_at || "").slice(0, 10);
  const sourceUrl = metadata.source_url || metadata.sourceUrl || metadata.url || metadata.original_url || "";
  const sourcePath = metadata.source_path || metadata.sourcePath || metadata.path || metadata.file_path || "";
  const lines = [`# ${title}`];
  if (author) lines.push(`作者：${author}`);
  if (date) lines.push(`发布时间：${date}`);
  if (sourceUrl) lines.push(`原文链接：${sourceUrl}`);
  if (sourcePath) lines.push(`来源文件：${sourcePath}`);
  lines.push(content);
  return lines.join("\n");
}

function parseMarkdownArticleBlocks(contextText) {
  const headings = [...contextText.matchAll(/^#\s+(.+)$/gm)];
  if (headings.length === 0) return [];

  return headings
    .map((heading, index) => {
      const next = headings[index + 1];
      const block = contextText.slice(heading.index, next ? next.index : contextText.length).trim();
      const author = lineValue(block, "作者");
      if (!AUTHORS.includes(author)) return null;

      const dateLine = lineValue(block, "发布时间");
      const date = dateLine.match(/\d{4}-\d{2}-\d{2}/)?.[0] || "";
      const title = heading[1].trim();
      if (isCandidateArticleBlock(block)) return null;
      const sourceUrl = lineValue(block, "原文链接");
      const sourcePath = lineValue(block, "来源文件");
      const body = cleanArticleBody(block);

      return {
        author,
        date,
        title,
        sourceUrl,
        sourcePath,
        sourceType: sourceMaterialKind({ author, sourceUrl, sourcePath }),
        excerpt: body.slice(0, 700),
        body,
      };
    })
    .filter(Boolean);
}

function isUserMaterialSource(source = {}) {
  const author = String(source.author || "").trim();
  return author === USER_SOURCE_AUTHOR
    || String(source.sourceUrl || "").startsWith("user-source://")
    || String(source.sourcePath || "").startsWith("user-sources/");
}

function isAuthorOriginalSource(source = {}) {
  const author = String(source.author || "").trim();
  return AUTHOR_SET.has(author) && !isUserMaterialSource(source);
}

function sourceMaterialKind(source = {}) {
  return isUserMaterialSource(source) ? "user_material" : "author_original";
}

function compactHistoryText(value, maxLength = 260) {
  const normalized = String(value || "")
    .replace(/\s+/g, " ")
    .trim();
  return normalized.length > maxLength ? `${normalized.slice(0, maxLength)}...` : normalized;
}

function buildAnswerText(
  question,
  ranked,
  articles,
  answerContext = question,
  productInputSummary = undefined,
  productDiagnosisInput = undefined,
  learningMemoryReminder = undefined,
) {
  if (articles.length === 0) {
    const productDiagnosis = productDiagnosisInput || buildProductDiagnosis(productInputSummary, answerContext, { hasSourceEvidence: false });
    const lines = [
      `问题：${question}`,
      "",
      "这次没有从本地知识库里找到足够相关的资料。 【缺少来源】",
    ];
    appendLearningMemoryReminder(lines, learningMemoryReminder);
    if (productDiagnosis) {
      lines.push("", "本轮诊断优先级：");
      lines.push(`1. 最先检查：${productDiagnosis.priority}`);
      lines.push(`2. 为什么先看它：${productDiagnosis.reason}`);
      lines.push(`3. 下一步补材料：${productDiagnosis.nextInputs}`);
      lines.push(`4. 证据边界：${productDiagnosis.boundary}`);
      if (productDiagnosis.decision) appendBusinessDecisionLines(lines, productDiagnosis.decision);
    }
    lines.push("可以换一种更具体的问法，例如加入“选品、关键词、主图、广告、新品、转化率”等关键词。");
    return lines.join("\n");
  }

  const lines = [`问题：${question}`, "", `我从本地知识库里找到了 ${articles.length} 篇相关资料。`];
  appendLearningMemoryReminder(lines, learningMemoryReminder);
  const productDiagnosis = productDiagnosisInput || buildProductDiagnosis(productInputSummary, answerContext, { hasSourceEvidence: true });
  if (productDiagnosis) {
    lines.push("", "本轮诊断优先级：");
    lines.push(`1. 最先检查：${productDiagnosis.priority}`);
    lines.push(`2. 为什么先看它：${productDiagnosis.reason}`);
    lines.push(`3. 下一步补材料：${productDiagnosis.nextInputs}`);
    lines.push(`4. 证据边界：${productDiagnosis.boundary}`);
    if (productDiagnosis.decision) appendBusinessDecisionLines(lines, productDiagnosis.decision);
  }

  const constrainedAdvice = buildSourceConstrainedAdvice(answerContext, ranked);

  lines.push("", "可执行结论：");
  constrainedAdvice.conclusions.forEach((item, index) => {
    lines.push(`${index + 1}. ${item} 【推断${index + 1}】`);
  });

  lines.push("", "执行顺序：");
  constrainedAdvice.steps.forEach((item, index) => {
    lines.push(`${index + 1}. ${item} 【行动${index + 1}】`);
  });

  if (ranked.length > 0) {
    lines.push("", "资料里最相关的判断：");
    ranked.slice(0, 5).forEach((point, index) => {
      lines.push(`${index + 1}. ${point.text}（${point.source.author}《${point.source.title}》） 【证据${index + 1}】`);
    });
  } else {
    lines.push("", "资料相关性不足：");
    lines.push("1. 这次虽然检索到资料，但没有找到足够相关、可直接引用的原文句子。 【缺少来源】");
    lines.push("2. 请换成更具体的问题，或补充产品、关键词、页面、广告数据后重查。");
  }

  const authorViews = buildAuthorViews(articles);
  if (authorViews.length > 0) {
    lines.push("", "作者视角：");
    authorViews.forEach((item) => lines.push(`- ${item}`));
  }

  lines.push("");
  lines.push("建议下一步：先按这些资料检查你的产品或页面，再把具体产品、关键词或 Listing 发进来继续追问。");
  return lines.join("\n");
}

function buildSourceConstrainedAdvice(question, ranked = []) {
  const type = detectAnswerType(question);
  const evidenceRows = Array.isArray(ranked) ? ranked.filter((item) => String(item?.text || "").trim()) : [];
  if (evidenceRows.length === 0) {
    return {
      conclusions: [
        "这次资料相关性不足，暂不能判断应该先做哪个动作。",
        "先补更直接的来源证据，再把行动建议放到验证任务里核对。",
      ],
      steps: [
        "先核对是否有直接回答本问题的作者原文",
        "再补产品数据和页面材料",
        "最后根据来源和数据决定执行顺序",
      ],
    };
  }

  if (type !== "visual") {
    return {
      conclusions: buildActionConclusion(question),
      steps: buildExecutionSteps(question),
    };
  }

  const constraint = buildSourceActionConstraint(question, evidenceRows);
  if (constraint.status === "caution") {
    return {
      conclusions: [
        "本轮来源明确提醒不建议先改主图，暂不能把主图点击入口作为固定第一结论。",
        "先核对评价、价格、页面承接和真实点击数据，再决定主图是否需要进入验证任务。",
        "如果后续数据证明点击入口确实是瓶颈，再把主图作为小实验处理。",
      ],
      steps: [
        "先核对来源里反向提醒的条件",
        "再看评价、价格和页面承接是否是当前瓶颈",
        "再补 CTR、CVR、核心词曝光和同屏竞品对比",
        "最后把是否改主图放进验证任务，而不是直接先改",
      ],
    };
  }
  if (constraint.status === "conflict") {
    return {
      conclusions: [
        "本轮来源对主图优先级有分歧，暂不能直接判断先改主图。",
        "先核对不同来源各自成立的条件，再用你的产品数据决定采纳哪一侧。",
        "主图可以作为待验证动作，但不能在冲突未解前当成固定第一步。",
      ],
      steps: [
        "先核对支持和反向来源的原文语境",
        "再补 CTR、CVR、评价、价格和页面承接数据",
        "再判断点击入口和页面承接哪个更像当前瓶颈",
        "最后只验证一个变量，避免同时改多个动作",
      ],
    };
  }

  const sourceLead = sourceActionLead(evidenceRows);
  return {
    conclusions: sourceLead
      ? [sourceLead, ...buildActionConclusion(question).filter((item) => item !== sourceLead).slice(0, 2)]
      : buildActionConclusion(question),
    steps: buildExecutionSteps(question),
  };
}

function sourceActionLead(evidenceRows = []) {
  for (const row of evidenceRows) {
    const text = String(row?.text || row?.quote || "").replace(/\s+/g, " ").trim();
    const match = text.match(/先[^。；;.!！?？\n]{4,90}/);
    if (match && /主图|点击率|首屏|卖点|利益点|图片/.test(match[0])) {
      return `本轮来源的直接动作是：${match[0]}。`;
    }
  }
  return "";
}

function buildSourceActionConstraint(question, evidenceRows = []) {
  if (detectAnswerType(question) !== "visual") return { status: "none" };
  const rows = Array.isArray(evidenceRows) ? evidenceRows : [];
  if (rows.length === 0) return { status: "insufficient" };
  const stance = sourceActionStance(rows, ["主图", "点击率"]);
  if (stance.hasCaution && stance.hasSupport) return { status: "conflict" };
  if (stance.hasCaution) return { status: "caution" };
  return { status: stance.hasSupport ? "support" : "none" };
}

function sourceActionStance(evidenceRows = [], concepts = []) {
  let hasSupport = false;
  let hasCaution = false;
  evidenceRows.forEach((item) => {
    const text = `${item.text || ""}\n${item.quote || ""}\n${item.source?.title || ""}\n${item.title || ""}`;
    concepts.forEach((concept) => {
      const stance = conflictStanceForText(text, concept);
      if (stance === "support") hasSupport = true;
      if (stance === "caution") hasCaution = true;
    });
  });
  return { hasSupport, hasCaution };
}

function appendBusinessDecisionLines(lines, decision) {
  lines.push("", "当前产品判断：");
  lines.push(`1. 判断：${decision.label}`);
  lines.push(`2. 理由：${decision.summary}`);
  const missing = Array.isArray(decision.missingData) && decision.missingData.length
    ? decision.missingData.map((item) => item.label).join("、")
    : "暂无关键缺口";
  lines.push(`3. 仍缺数据：${missing}`);
  lines.push(`4. 证据边界：${decision.boundary}`);
}

function appendLearningMemoryReminder(lines, reminder) {
  const items = Array.isArray(reminder?.items) ? reminder.items.filter((item) => item?.excerpt || item?.title) : [];
  if (items.length === 0) return;
  lines.push("", "本地学习档案提醒：");
  items.slice(0, 3).forEach((item, index) => {
    const title = item.title || "学习档案";
    const excerpt = item.excerpt || "这条学习档案暂时没有摘要。";
    const kindLabel = item.memoryKindLabel || "历史学习笔记";
    const usage = ["business_material", "experiment_review"].includes(item.memoryKind)
      ? "（只用于验证你的业务，不是作者原文证据）"
      : "（不是作者原文证据）";
    lines.push(`${index + 1}. 你之前沉淀过「${title}」[${kindLabel}]：${excerpt}${usage}`);
  });
  if (reminder.alignment?.message) {
    lines.push(`校准：${reminder.alignment.message}`);
  }
  lines.push(`边界：${reminder.boundary || "这些是你保存过的学习档案提醒，不是作者原文证据；本轮引用仍只来自作者资料。"}`);
}

function buildActionConclusion(question) {
  const type = detectAnswerType(question);
  if (type === "visual") {
    return [
      "先把主图当成点击率入口处理；没有点击率，后面的转化率分析意义会变弱。",
      "不要只做“更好看”的图，要做搜索结果里能被一眼识别的差异化图。",
      "转化率要和副图、文案、对比图、场景图一起看，不能只盯广告数据。",
    ];
  }
  if (type === "product") {
    return [
      "先判断市场有没有足够需求，再判断自己有没有进入空间。",
      "选品不是只看热度，要同时看竞争、利润、退货、评分、新品进入机会。",
      "如果一个产品无法复用你的能力或资产，就不要轻易投入重资源。",
    ];
  }
  if (type === "listing") {
    return [
      "先把关键词词库做清楚，再决定标题、Search Terms、五点和图片分别承接哪些词。",
      "不要为了塞词牺牲可读性，标题前段要优先承接最相关、最有购买意图的词。",
      "收录不是一次性动作，广告报告和后台数据要反过来持续修正文案和词库。",
    ];
  }
  if (type === "ads") {
    return [
      "广告不是救命稻草，它只会放大 Listing 和产品基本面的好坏。",
      "新品期先跑出有效词和有效渠道，再集中预算推真正有价值的词。",
      "预算、竞价、位置、否词要围绕阶段目标调整，不要盲目高举高打。",
    ];
  }
  return [
    "先把问题拆成市场、产品、页面、流量四层，不要直接跳到单一动作。",
    "每个动作都要能回到资料来源验证，避免只凭经验判断。",
    "追问时带上具体产品、关键词、页面或广告数据，答案会更可执行。",
  ];
}

function buildExecutionSteps(question) {
  const type = detectAnswerType(question);
  if (type === "visual") {
    return ["先看主图点击率", "再看副图是否解释清楚差异和使用场景", "再看五点和 A+ 是否补足信任", "最后再调广告和预算"];
  }
  if (type === "product") {
    return ["市场容量", "头部垄断程度", "新品占比", "利润和推广成本", "退货率与评分风险", "差异化机会"];
  }
  if (type === "listing") {
    return ["整理竞品和搜索词", "筛掉不相关词", "分配标题和 Search Terms", "用五点承接痛点", "用广告报告继续修正"];
  }
  if (type === "ads") {
    return ["确认 Listing 基本面", "跑自动或探索广告找词", "筛出能转化的词", "集中预算推重点词", "用否词和位置调整控制浪费"];
  }
  return ["明确问题类型", "找对应来源", "提炼判断标准", "形成检查清单", "用真实数据回测"];
}

function buildProductDiagnosis(summary, answerContext, options = {}) {
  if (!summary || !Array.isArray(summary.facts) || summary.facts.length === 0) return null;
  const facts = flattenProductFacts(summary);
  const factText = facts.join(" ");
  const type = detectAnswerType(`${answerContext}\n${factText}`);
  const visual = factsForLabels(summary, /主图|视觉/);
  const metrics = uniqueRawItems([...factsForLabels(summary, /点击率|转化率|数据/), ...metricLikeFacts(facts)]);
  const listing = factsForLabels(summary, /Listing|页面/);
  const ads = uniqueRawItems([...factsForLabels(summary, /广告|流量/), ...adLikeFacts(facts)]);
  const keywords = factsForLabels(summary, /关键词|竞品/);
  const missing = Array.isArray(summary.missing) ? summary.missing.filter(Boolean).slice(0, 5) : [];
  const ctr = extractPercentMetric(metrics, /\bctr\b|点击率/i);
  const cvr = extractPercentMetric(metrics, /\bcvr\b|转化率/i);
  const acos = extractPercentMetric(ads.length > 0 ? ads : facts, /\bacos\b/i);
  const factClips = [
    firstFact(visual),
    metricFact("CTR", ctr),
    metricFact("CVR", cvr),
    metricFact("ACOS", acos),
    firstFact(listing),
    firstFact(keywords),
  ].filter(Boolean);
  const factBasis = factClips.length > 0 ? `你提供的信息里有：${factClips.slice(0, 6).join("；")}。` : "你已经补充了部分产品信息。";
  const nextInputs = missing.length > 0 ? missing.join("、") : fallbackNextInputs(type);
  const hasSourceEvidence = options.hasSourceEvidence !== false;
  const boundary = hasSourceEvidence
    ? "以上优先级来自用户输入 + 本地资料的诊断推断；用户输入不是原文证据，仍要用后台数据和页面截图复核。"
    : "以上优先级只来自用户输入和通用检查框架；这次没有命中本地资料，必须补充页面截图、后台数据或换更具体的问题复核。";
  const sourceActionConstraint = options.sourceActionConstraint || {};

  if (sourceActionConstraint.status === "caution") {
    return {
      priority: "先核对来源反向提醒，暂不能直接判定先改主图。",
      reason: `${factBasis}但本轮作者来源明确不建议先改主图；先把评价、价格、页面承接和真实点击数据核对清楚，再决定主图是否进入验证任务。`,
      nextInputs,
      boundary,
    };
  }

  if (sourceActionConstraint.status === "conflict") {
    return {
      priority: "先核对来源分歧，暂不能直接判定先改主图。",
      reason: `${factBasis}本轮来源对主图优先级有分歧；先补 CTR、CVR、评价、价格、页面承接和同屏竞品对比，再决定采纳哪一侧。`,
      nextInputs,
      boundary,
    };
  }

  if (visual.length > 0 && ctr !== null && ctr < 0.4) {
    return {
      priority: "先改搜索结果里的主图点击入口。",
      reason: `${factBasis}CTR 已经偏低，说明问题更可能先发生在点击入口；这时直接看 CVR 或继续加广告，容易把入口问题误判成页面问题。`,
      nextInputs,
      boundary,
    };
  }

  if (ads.length > 0 && acos !== null && acos >= 40 && (visual.length > 0 || listing.length > 0)) {
    return {
      priority: "先暂停把广告当主解法，回头检查主图和 Listing 基本面。",
      reason: `${factBasis}ACOS 已经偏高，广告更像是在放大页面或点击入口的问题；先确认主图、标题、五点和评价基础，再决定预算怎么调。`,
      nextInputs,
      boundary,
    };
  }

  if (listing.length > 0 && keywords.length > 0 && metrics.length === 0) {
    return {
      priority: "先补点击率、转化率和主要流量来源，再判断 Listing 该改哪里。",
      reason: `${factBasis}现在有页面和关键词线索，但缺少结果数据；没有 CTR/CVR 就很难区分是搜索结果点击问题、页面说服问题，还是关键词不准。`,
      nextInputs,
      boundary,
    };
  }

  if (type === "ads" && ads.length > 0) {
    return {
      priority: "先判断广告是在探索有效词，还是在放大低质量流量。",
      reason: `${factBasis}广告数据不能单独看，要同时对照关键词、Listing 和转化数据，否则容易只做调预算而没有解决根因。`,
      nextInputs,
      boundary,
    };
  }

  return {
    priority: defaultDiagnosisPriority(type),
    reason: `${factBasis}这些信息还不足以直接下最终结论，先按资料里的检查顺序把入口、页面、流量和竞争对照拆开验证。`,
    nextInputs,
    boundary,
  };
}

function buildDiagnosisPanel(summary, diagnosis, answerContext) {
  if (!summary || !Array.isArray(summary.facts) || summary.facts.length === 0 || !diagnosis) return undefined;
  const facts = flattenProductFacts(summary);
  const factText = facts.join(" ");
  const type = detectAnswerType(`${answerContext}\n${factText}`);
  const visual = factsForLabels(summary, /主图|视觉/);
  const metrics = uniqueRawItems([...factsForLabels(summary, /点击率|转化率|数据/), ...metricLikeFacts(facts)]);
  const listing = factsForLabels(summary, /Listing|页面/);
  const ads = uniqueRawItems([...factsForLabels(summary, /广告|流量/), ...adLikeFacts(facts)]);
  const keywords = factsForLabels(summary, /关键词|竞品/);
  const missing = Array.isArray(summary.missing) ? summary.missing.filter(Boolean).slice(0, 8) : [];
  const tracks = [];
  const addTrack = (id, label, level, why, checks, prompt) => {
    const safeChecks = uniqueSafeList(checks, 5, 120);
    if (safeChecks.length === 0) return;
    tracks.push({
      id,
      label,
      level,
      why: compactGraphLabel(why, 160),
      checks: safeChecks.map((item, index) => ({ id: `${id}:${index}`, label: item })),
      prompt: compactGraphLabel(prompt, 360),
    });
  };

  addTrack(
    "visual-entry",
    "主图入口检查",
    type === "visual" || visual.length > 0 ? "优先" : "观察",
    visual.length > 0 ? `已提供：${firstFact(visual)}` : "先确认搜索结果里用户是否愿意点进来。",
    [
      "把你的主图和前三个竞品主图放在同一行对比",
      "记录核心关键词下的 CTR、曝光和点击变化",
      "检查主图是否一眼能看出差异、用途或套装价值",
      ...missing.filter((item) => /主图|截图|竞品|卖点/.test(item)),
    ],
    "我补充了主图入口检查结果：。请判断下一步是改主图、补副图，还是先看 Listing。",
  );

  addTrack(
    "listing-bridge",
    "Listing 承接检查",
    listing.length > 0 ? "待验证" : "补材料",
    listing.length > 0 ? `已提供：${firstFact(listing)}` : "点击进来之后，要确认标题、五点、A+ 和评价是否承接住主图承诺。",
    [
      "核对标题前半段是否承接核心关键词和购买意图",
      "检查五点是否解释差异、场景、规格和信任理由",
      "补充价格、评价数量、星级和主要差评点",
      ...missing.filter((item) => /价格|评价|标题|五点|Listing|A\+/.test(item)),
    ],
    "我补充了 Listing 承接信息：。请判断现在是先改标题五点，还是先补图片/评价证据。",
  );

  addTrack(
    "ad-waste",
    "广告浪费检查",
    ads.length > 0 ? "高风险" : "观察",
    ads.length > 0 ? `已提供：${firstFact(ads)}` : "广告要看它是在探索有效词，还是放大低质量点击。",
    [
      "列出主要花费词、点击、订单、ACOS 和 CVR",
      "区分 SP、SBV、自动广告和精准词的表现",
      "检查高花费无订单词是否需要降价、否词或暂停",
      ...missing.filter((item) => /广告|ACOS|预算|花费|流量/.test(item)),
    ],
    "我补充了广告浪费检查结果：。请判断应该先调预算、否词，还是先改页面基本面。",
  );

  addTrack(
    "keyword-competitor",
    "关键词/竞品对照",
    keywords.length > 0 ? "待验证" : "补材料",
    keywords.length > 0 ? `已提供：${firstFact(keywords)}` : "关键词和竞品决定你比较的是哪一组真实货架。",
    [
      "确认核心关键词的搜索结果截图和前三名竞品 ASIN",
      "记录竞品主图、价格、评价数量、卖点和促销方式",
      "检查你的 Listing 是否承接核心词背后的购买任务",
      ...missing.filter((item) => /关键词|竞品|ASIN|类目|搜索/.test(item)),
    ],
    "我补充了关键词和竞品对照：。请判断我的差异化入口应该放在主图、价格、套装还是文案。",
  );

  if (missing.length > 0) {
    addTrack(
      "missing-data",
      "需要补充的数据",
      "缺口",
      "这些缺口会影响下一轮判断的准确性。",
      missing,
      `我补充了这些缺失材料：${missing.slice(0, 5).join("、")}。请重新判断最先改哪一块。`,
    );
  }

  return {
    summary: "把本轮诊断拆成可执行排查项，勾选状态只保存在当前对话。",
    priority: diagnosis.priority,
    reason: diagnosis.reason,
    tracks: tracks.slice(0, 5),
    caution: diagnosis.boundary,
  };
}

function flattenProductFacts(summary) {
  return summary.facts.flatMap((section) => (Array.isArray(section.items) ? section.items : [])).filter(Boolean);
}

function factsForLabels(summary, pattern) {
  return summary.facts
    .filter((section) => productSectionMatchesPattern(section, pattern))
    .flatMap((section) => (Array.isArray(section.items) ? section.items : []))
    .filter(Boolean);
}

function productSectionFallbackLabel(id) {
  const labels = {
    visual: "主图/视觉",
    metrics: "点击率/转化率数据",
    listing: "Listing/页面",
    ads: "广告/流量",
    keywords: "关键词/竞品",
  };
  return labels[String(id || "").trim()] || "产品信息";
}

function productSectionMatchesPattern(section, pattern) {
  const target = `${section?.label || ""}\n${section?.id || ""}\n${productSectionFallbackLabel(section?.id)}`;
  pattern.lastIndex = 0;
  return pattern.test(target);
}

function metricLikeFacts(items) {
  return uniqueRawItems(items.filter((item) => /\bctr\b|\bcvr\b|点击率|转化率|session|sessions|曝光|impression/i.test(String(item || ""))));
}

function adLikeFacts(items) {
  return uniqueRawItems(items.filter((item) => /\bacos\b|广告|ppc|sp\b|sbv|预算|竞价|bid|campaign/i.test(String(item || ""))));
}

function uniqueRawItems(items) {
  const seen = new Set();
  const output = [];
  for (const item of items || []) {
    const text = String(item || "").trim();
    if (!text || seen.has(text)) continue;
    seen.add(text);
    output.push(text);
  }
  return output;
}

function extractPercentMetric(items, labelPattern) {
  const labels = metricLabelHints(labelPattern);
  const explicitLabels = labels.filter((label) => isExplicitMetricLabel(label));
  const broadLabels = labels.filter((label) => !isExplicitMetricLabel(label));
  const explicitMatch = findPercentMetricNearLabels(items, explicitLabels, labelPattern);
  if (explicitMatch !== null) return explicitMatch;

  const combinedMatch = findCombinedChinesePercentMetric(items, labelPattern);
  if (combinedMatch !== null) return combinedMatch;

  const broadMatch = findPercentMetricNearLabels(items, broadLabels, labelPattern);
  if (broadMatch !== null) return broadMatch;

  for (const item of items) {
    const text = String(item || "");
    labelPattern.lastIndex = 0;
    if (!labelPattern.test(text)) continue;
    if (hasConflictingExplicitMetricLabel(text, labelPattern)) continue;
    const percent = text.match(/(\d+(?:\.\d+)?)\s*%/);
    if (percent) return Number(percent[1]);
  }
  return null;
}

function findPercentMetricNearLabels(items, labels, labelPattern) {
  for (const item of items) {
    const text = String(item || "");
    for (const label of labels) {
      const escaped = escapeRegExp(label);
      const afterLabel = text.match(new RegExp(`${escaped}([^\\d%]{0,16})(\\d+(?:\\.\\d+)?)\\s*%`, "i"));
      if (afterLabel && !hasConflictingExplicitMetricLabel(afterLabel[1], labelPattern)) return Number(afterLabel[2]);
      const beforeLabel = text.match(new RegExp(`(\\d+(?:\\.\\d+)?)\\s*%([^\\n,，;；]{0,16})${escaped}`, "i"));
      if (beforeLabel && !hasConflictingExplicitMetricLabel(beforeLabel[2], labelPattern)) return Number(beforeLabel[1]);
    }
  }
  return null;
}

function findCombinedChinesePercentMetric(items, labelPattern) {
  const source = String(labelPattern || "").toLowerCase();
  const wantsCtr = source.includes("ctr") || source.includes("点击率");
  const wantsCvr = source.includes("cvr") || source.includes("转化率");
  if (!wantsCtr && !wantsCvr) return null;
  for (const item of items) {
    const text = String(item || "");
    if (!/点击率/.test(text) || !/转化率/.test(text)) continue;
    if (/\bctr\b|\bcvr\b/i.test(text)) continue;
    const matches = [...text.matchAll(/(\d+(?:\.\d+)?)\s*%/g)].map((match) => Number(match[1]));
    if (matches.length < 2) continue;
    const orderedMetricNames = [...text.matchAll(/点击率|转化率/g)].map((match) => match[0]);
    if (/分别/.test(text) && orderedMetricNames.length >= 2) {
      const target = wantsCtr ? "点击率" : "转化率";
      const index = orderedMetricNames.indexOf(target);
      if (index >= 0 && matches[index] !== undefined) return matches[index];
    }
    if (wantsCtr) return matches[0];
    if (wantsCvr) return matches[1];
  }
  return null;
}

function isExplicitMetricLabel(label) {
  return /^[a-z0-9]+$/i.test(label);
}

function hasConflictingExplicitMetricLabel(text, labelPattern) {
  const source = String(labelPattern || "").toLowerCase();
  const value = String(text || "");
  if ((source.includes("cvr") || source.includes("转化率")) && /\bctr\b/i.test(value)) return true;
  if ((source.includes("ctr") || source.includes("点击率")) && /\bcvr\b/i.test(value)) return true;
  return false;
}

function metricLabelHints(labelPattern) {
  const source = String(labelPattern || "").toLowerCase();
  if (source.includes("ctr") || source.includes("点击率")) return ["ctr", "点击率"];
  if (source.includes("cvr") || source.includes("转化率")) return ["cvr", "转化率"];
  if (source.includes("acos")) return ["acos"];
  return [];
}

function firstFact(items) {
  const text = compactGraphLabel(items.find(Boolean) || "", 70);
  return text || "";
}

function metricFact(label, value) {
  return value === null ? "" : `${label} ${value}%`;
}

function fallbackNextInputs(type) {
  return buildMissingInputs(type, []).slice(0, 5).join("、");
}

function defaultDiagnosisPriority(type) {
  if (type === "visual") return "先把主图点击入口、页面解释力和广告流量拆开检查。";
  if (type === "ads") return "先确认 Listing 基本面，再判断广告预算和关键词。";
  if (type === "listing") return "先检查关键词是否被标题、五点、图片和 Search Terms 正确承接。";
  if (type === "product") return "先确认市场需求、竞争强度和进入理由。";
  return "先把问题拆成入口、页面、流量和产品力四层。";
}

function buildAuthorViews(articles) {
  const authors = new Set(articles.map((article) => article.author));
  const views = [];
  if (authors.has("张子卿")) views.push("张子卿更适合看“做不做、怎么判断、实盘取舍”。");
  if (authors.has("飞翔的波波")) views.push("飞翔的波波更适合看“怎么推、怎么拆数据、怎么做广告和选品分析”。");
  if (authors.has("跨境电商长期主义")) views.push("跨境电商长期主义更适合看“怎么沉淀系统、词库、页面、视觉和长期复用能力”。");
  return views;
}

function describeIntent(type) {
  const descriptions = {
    visual: {
      type,
      label: "视觉转化诊断",
      description: "你现在更像是在判断主图、副图、页面视觉和转化之间的优先级。",
    },
    product: {
      type,
      label: "选品决策判断",
      description: "你现在更像是在判断一个产品或市场是否值得投入。",
    },
    listing: {
      type,
      label: "Listing 与关键词承接",
      description: "你现在更像是在把搜索词、标题、五点、Search Terms 和页面内容分工。",
    },
    ads: {
      type,
      label: "广告推广诊断",
      description: "你现在更像是在判断广告、预算、关键词和 Listing 基本面的配合。",
    },
    general: {
      type,
      label: "经营问题拆解",
      description: "你现在的问题需要先拆成市场、产品、页面和流量几层再判断。",
    },
  };
  return descriptions[type] || descriptions.general;
}

function buildFallbackTakeaway(type) {
  if (type === "visual") return "先把主图点击率、页面解释力和转化证据分开检查。";
  if (type === "product") return "先判断需求、竞争、利润和可复用能力，再决定是否投入。";
  if (type === "listing") return "先把关键词变成用户任务，再分配到页面不同位置承接。";
  if (type === "ads") return "先确认 Listing 和产品基本面，再用广告验证有效词。";
  return "先把问题拆层，再回到来源资料和真实数据验证。";
}

function buildMissingInputs(type, sources) {
  const common = ["具体产品或类目", "核心关键词或竞品链接"];
  const sourceHint = sources.length === 0 ? ["更明确的业务场景或资料范围"] : [];
  if (type === "visual") return ["你的主图/副图截图", "当前点击率和转化率", ...common, ...sourceHint];
  if (type === "product") return ["目标售价和成本", "核心关键词搜索量", "头部竞品销量和评分", ...common, ...sourceHint];
  if (type === "listing") return ["当前标题、五点和 Search Terms", "关键词词库", "广告搜索词报告", ...common, ...sourceHint];
  if (type === "ads") return ["广告类型和预算", "CPC、ACOS、转化率", "已跑出的搜索词", ...common, ...sourceHint];
  return [...common, "你最想先解决的业务目标", ...sourceHint];
}

function buildFollowUps(type) {
  if (type === "visual") {
    return [
      "我把具体产品和主图发你，帮我判断先改哪一块。",
      "如果主图点击率低但页面转化还行，应该怎么排查？",
      "帮我按主图、副图、Listing、广告顺序做一张检查清单。",
    ];
  }
  if (type === "product") {
    return [
      "我给你一个产品想法，帮我判断是否值得做。",
      "这个类目应该先看哪些关键词和竞品数据？",
      "帮我做一个新品 No-Go 检查清单。",
    ];
  }
  if (type === "listing") {
    return [
      "我把 Listing 发你，帮我判断关键词承接是否合理。",
      "标题、五点、Search Terms 应该分别放哪些词？",
      "帮我把这些关键词翻译成用户购买任务。",
    ];
  }
  if (type === "ads") {
    return [
      "我把广告数据发你，帮我判断先降预算还是先改页面。",
      "新品广告前 14 天应该看哪些指标？",
      "自动广告跑出来的词应该怎么筛选和否词？",
    ];
  }
  return [
    "我把具体产品、关键词或页面发你，帮我继续判断。",
    "这个问题可以拆成哪几个检查步骤？",
    "基于这些资料，帮我生成一张可执行清单。",
  ];
}

function relatedEvidenceIndexes(text, rankedEvidence) {
  const concepts = detectGraphConcepts(text);
  if (concepts.length === 0) return [];
  const indexes = [];
  for (const item of rankedEvidence) {
    if (concepts.some((concept) => textMentionsConcept(item.quote, concept))) indexes.push(item.sourceIndex);
  }
  return [...new Set(indexes)];
}

function buildValidationPrompt(type) {
  if (type === "visual") return "需要用你的主图/副图截图、点击率、转化率和页面数据验证。";
  if (type === "product") return "需要用具体产品、关键词搜索量、竞品销量、利润和退货风险验证。";
  if (type === "listing") return "需要用当前 Listing、关键词词库、收录情况和广告搜索词报告验证。";
  if (type === "ads") return "需要用广告预算、CPC、ACOS、转化率和搜索词报告验证。";
  return "需要用你的具体场景、目标和业务数据验证。";
}

function detectAnswerType(question) {
  const value = String(question || "").toLowerCase();
  if (/主图|图片|视觉|点击率|转化率/.test(value)) return "visual";
  if (/选品|值不值得做|能不能做|做不做|市场容量|新品/.test(value)) return "product";
  if (/listing|文案|关键词|收录|标题|search term|五点|bullet/.test(value)) return "listing";
  if (/广告|推广|投放|acos|cpc|竞价|预算/.test(value)) return "ads";
  return "general";
}

function fallbackPoints(articles) {
  return articles.slice(0, 3).map((article) => ({
    text: article.excerpt.split("\n").find(Boolean) || article.title,
    source: article,
  }));
}

export function buildSourceContextFromArticle(source = {}, articleText = "", options = {}) {
  const rawArticle = String(articleText || "");
  const body = cleanArticleBody(rawArticle);
  const quote = String(options.quote || source.excerpt || "").trim();
  const metadata = sourceContextMetadata(source, rawArticle);
  const base = {
    ...metadata,
    identity: "作者原文",
    quote: quote ? compactGraphLabel(quote, 360) : "",
  };

  if (!body) {
    return {
      ...base,
      status: "missing_source",
      canUseAsEvidence: false,
      before: "",
      match: "",
      after: "",
      contextText: "",
      reason: "没有找到可读取的本地原文内容。",
    };
  }

  const located = locateQuoteContext(body, quote);
  if (!located) {
    return {
      ...base,
      status: "not_located",
      canUseAsEvidence: false,
      before: "",
      match: "",
      after: "",
      contextText: compactGraphLabel(body, 900),
      reason: "未定位到原文上下文；保留来源身份，但不猜测引用位置。",
    };
  }

  return {
    ...base,
    status: "located",
    canUseAsEvidence: true,
    before: compactGraphLabel(located.before, 520),
    match: compactGraphLabel(located.match, 900),
    after: compactGraphLabel(located.after, 520),
    contextText: [located.before, located.match, located.after].filter(Boolean).join("\n\n"),
    reason: "",
  };
}

function sourceContextMetadata(source = {}, articleText = "") {
  const title = String(articleText || "").match(/^#\s+(.+)$/m)?.[1]?.trim() || source.title || "";
  const dateLine = lineValue(articleText, "发布时间");
  const date = dateLine.match(/\d{4}-\d{2}-\d{2}/)?.[0] || source.date || "";
  return {
    author: lineValue(articleText, "作者") || source.author || "",
    date,
    title,
    sourceUrl: lineValue(articleText, "原文链接") || source.sourceUrl || "",
    sourcePath: lineValue(articleText, "来源文件") || source.sourcePath || "",
  };
}

function locateQuoteContext(body, quote) {
  const paragraphs = sourceContextParagraphs(body);
  if (paragraphs.length === 0) return null;
  const candidates = sourceContextQuoteCandidates(quote);

  for (const candidate of candidates) {
    const index = paragraphs.findIndex((paragraph) => paragraph.includes(candidate));
    if (index !== -1) return paragraphWindow(paragraphs, index);
  }

  for (const candidate of candidates) {
    const normalizedCandidate = normalizeSourceContextText(candidate);
    if (normalizedCandidate.length < 24) continue;
    const index = paragraphs.findIndex((paragraph) => normalizeSourceContextText(paragraph).includes(normalizedCandidate));
    if (index !== -1) return paragraphWindow(paragraphs, index);
  }

  return null;
}

function sourceContextParagraphs(body) {
  return String(body || "")
    .split(/\n+/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function sourceContextQuoteCandidates(quote) {
  const raw = String(quote || "").trim();
  if (!raw) return [];
  const lines = raw
    .split(/\n+/)
    .map((item) => item.trim())
    .filter((item) => item.length >= 18);
  const collapsed = raw.replace(/\s+/g, " ").trim();
  const sentences = splitSentences(raw);
  return uniqueOrdered([
    raw,
    ...lines,
    ...sentences,
    collapsed,
    collapsed.slice(0, 260),
    collapsed.slice(0, 180),
    collapsed.slice(0, 120),
  ].filter((item) => item && item.length >= 18), 12);
}

function normalizeSourceContextText(value) {
  return String(value || "").replace(/\s+/g, " ").trim();
}

function paragraphWindow(paragraphs, index) {
  return {
    before: paragraphs[index - 1] || "",
    match: paragraphs[index] || "",
    after: paragraphs[index + 1] || "",
  };
}

function lineValue(block, label) {
  const regex = new RegExp(`^${escapeRegExp(label)}：(.+)$`, "m");
  return block.match(regex)?.[1]?.trim() ?? "";
}

function isCandidateArticleBlock(block) {
  return /(?:^|\n)来源状态：.*(?:候选|待确认|不能直接当成已采纳证据)/.test(String(block || ""));
}

function cleanArticleBody(block) {
  return block
    .split("\n")
    .filter((line) => {
      const trimmed = line.trim();
      if (!trimmed) return false;
      if (trimmed.startsWith("作者：")) return false;
      if (trimmed.startsWith("发布时间：")) return false;
      if (trimmed.startsWith("原文链接：")) return false;
      if (trimmed.startsWith("来源文件：")) return false;
      if (trimmed.startsWith("来源状态：")) return false;
      if (/^.+?\s+\d{4}-\d{2}-\d{2}\s+.+?:\s+#\s+/.test(trimmed)) return false;
      if (trimmed.startsWith("# ")) return false;
      return true;
    })
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function buildQuestionProfile(text) {
  const allTerms = tokenizeQuestion(text);
  const domainTerms = detectDomainTerms(text);
  return { allTerms, domainTerms, inDomain: hasAmazonLearningIntent(text, domainTerms) };
}

function tokenizeQuestion(text) {
  const normalized = String(text || "").toLowerCase();
  const latin = normalized.match(/[a-z0-9]+/g) ?? [];
  const cjk = [...normalized.matchAll(/[\u3400-\u9fff]{2,}/g)].flatMap((match) => cjkTerms(match[0]));
  return [
    ...new Set(
      [...latin, ...cjk].filter((item) => item.length >= 2 && !GENERIC_CJK_TERMS.has(item)),
    ),
  ];
}

function detectDomainTerms(text) {
  const value = String(text || "").toLowerCase();
  const terms = [];
  for (const group of DOMAIN_EXPANSIONS) {
    if (group.pattern.test(value)) terms.push(...group.terms);
  }
  return [...new Set(terms.map((term) => term.toLowerCase()))];
}

function hasAmazonLearningIntent(text, domainTerms = []) {
  if (Array.isArray(domainTerms) && domainTerms.length > 0) return true;
  const value = String(text || "");
  return (
    /亚马逊|amazon|listing|asin|fba|acos|cpc|ctr|cvr|search term|buy box/i.test(value) ||
    /选品|新品|产品|卖不动|卖不出去|关键词|词库|搜索词|主图|首图|副图|图片|点击率|转化率|广告|推广|投放|价格|评价|竞品|类目|利润|销量|库存|页面|标题|五点|流量|运营|店铺|变体/.test(value) ||
    isAuthorComparisonRequest(value)
  );
}

function cjkTerms(text) {
  const terms = [];
  if (text.length <= 8) terms.push(text);
  for (let size = 2; size <= 4; size += 1) {
    for (let index = 0; index <= text.length - size; index += 1) {
      terms.push(text.slice(index, index + size));
    }
  }
  return terms;
}

function splitSentences(text) {
  return String(text || "")
    .split(/(?<=[。！？!?])|\n+/u)
    .map((item) => item.trim())
    .filter((item) => item.length >= 18)
    .map((item) => (item.length > 220 ? item.slice(0, 220) : item));
}

function scoreText(text, profile) {
  const lower = text.toLowerCase();
  if (String(text || "").length <= 320 && isTopicAbsenceStatement(lower, profile)) return 0;
  if (!profile?.inDomain) return 0;
  let score = 0;
  let domainMatches = 0;

  for (const term of profile.domainTerms) {
    if (!lower.includes(term)) continue;
    domainMatches += 1;
    score += term.length >= 4 ? 7 : 5;
  }

  if (profile.domainTerms.length > 0 && domainMatches === 0) return 0;

  for (const term of profile.allTerms) {
    if (lower.includes(term)) score += term.length >= 4 ? 3 : 1;
  }
  if (/判断|检查|分析|核心|关键词|广告|转化|主图|选品|新品|流量/.test(text)) score += 1;
  return score;
}

function isTopicAbsenceStatement(lowerText, profile) {
  const value = String(lowerText || "");
  if (!/(不|未|没有|没|无).{0,8}(讨论|涉及|提到|讲|展开|分析|研究|解决)/.test(value)) return false;
  const domainTerms = Array.isArray(profile?.domainTerms) ? profile.domainTerms : [];
  if (domainTerms.length === 0) return false;
  const matchedTerms = domainTerms.filter((term) => value.includes(String(term || "").toLowerCase()));
  return matchedTerms.length >= Math.min(2, domainTerms.length);
}

function dedupeSentences(rows) {
  const seen = new Set();
  const result = [];
  for (const row of rows) {
    const key = row.text.slice(0, 60);
    if (seen.has(key)) continue;
    seen.add(key);
    result.push(row);
  }
  return result;
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
