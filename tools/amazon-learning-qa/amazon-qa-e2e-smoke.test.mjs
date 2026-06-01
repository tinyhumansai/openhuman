import assert from "node:assert/strict";
import test from "node:test";

import {
  FINAL_ACCEPTANCE_SCENARIOS,
  TOPIC_SWITCH_ACCEPTANCE_SCENARIO,
  validateAmazonQaFinalAcceptance,
  validateAmazonQaSmoke,
} from "./amazon-qa-e2e-smoke.mjs";

function statusPayload(overrides = {}) {
  return {
    health: {
      documents: 1779,
      chunks: 14597,
      embeddedChunks: 14597,
      vectorCoveragePercent: 100,
      sourceTree: {
        ingestedDocuments: 1779,
        manifestDocuments: 1779,
        queuedJobs: 10,
        failedJobs: 0,
        trees: 184,
        summaries: 58,
      },
    },
    readiness: {
      searchStatus: "ready",
      citationStatus: "ready",
      learningStatus: "processing",
    },
    ...overrides,
  };
}

function askPayload(question, overrides = {}) {
  return {
    question,
    sessionId: "smoke-session",
    answer: `问题：${question}\n\n本轮回答。`,
    answerGeneration: {
      mode: "template_fallback",
      model: "stable-template",
      boundary: "模板回答只整理本轮检索到的来源和摘录；引用仍需回到下方来源卡片核对。",
    },
    sources: [
      {
        author: "飞翔的波波",
        date: "2025-08-08",
        title: "新手必看！亚马逊测款实战指南",
        excerpt: "点击率反映主图吸引力；转化率反映产品、价格、文案的综合接受度。",
      },
    ],
    graph: {
      nodes: [
        { id: "question", type: "question", label: question },
        { id: "concept:主图", type: "concept", label: "主图" },
      ],
      edges: [{ from: "question", to: "concept:主图", type: "mentions" }],
    },
    topicSourceTree: {
      status: "ready",
      sources: [{ sourceIndex: 0, title: "新手必看！亚马逊测款实战指南" }],
    },
    notebookGuide: { status: "source_backed", briefing: [{ label: "主图", text: "先看点击入口。" }] },
    synthesisAnswer: { status: "source_backed", points: [{ label: "主图", text: "先看点击入口。" }] },
    sourceDecisionTable: {
      title: "来源决策表",
      status: "needs_data",
      summary: "把作者原文拆成可复核的行动判断。",
      rows: [
        {
          sourceIndex: 0,
          quote: "点击率反映主图吸引力。",
          supports: "可支持先看主图点击入口。",
          cannotProve: "不能证明改主图一定提升转化。",
          validation: "补充 CTR 和 CVR。",
          canUseAsEvidence: false,
          sourceCanUseAsEvidence: true,
        },
      ],
      boundary: "来源决策表是系统整理的决策辅助，不是新的作者原文证据。",
    },
    learningQueue: { items: [{ id: "queue:evidence", label: "核对证据" }] },
    knowledgeGapRadar: {
      title: "知识缺口雷达",
      status: "needs_data",
      summary: "下一步补产品数据。",
      gaps: [{ id: "gap:business-data", label: "缺少你的产品数据", canUseAsEvidence: false }],
      metrics: { sourceCount: 1, evidenceCount: 1, authorCount: 1 },
      boundary: "不改变作者原文证据边界。",
    },
    nextBestSource: {
      title: "下一步资料选择",
      status: "needs_data",
      summary: "先读关键来源，再补产品数据。",
      criteria: [{ id: "source-evidence", label: "先有作者原文", status: "ready", detail: "本轮已定位作者原文证据。" }],
      recommended: {
        id: "next-source:0",
        kind: "source",
        label: "新手必看！亚马逊测款实战指南",
        sourceIndex: 0,
        reason: "这是本轮回答实际引用到的关键作者原文。",
        sourceCanUseAsEvidence: true,
        canUseAsEvidence: false,
      },
      alternatives: [],
      boundary: "下一步资料选择只安排阅读、复核和补材料顺序；推荐理由是系统整理，不是新的作者原文证据。",
    },
    messages: [
      { role: "user", content: "主图视觉点击率转化率怎么优化？" },
      { role: "assistant", content: "第一轮回答", sources: [{}], graph: { nodes: [{}], edges: [] }, knowledgeGapRadar: { gaps: [{}] }, nextBestSource: { recommended: { label: "来源" } } },
      { role: "user", content: question },
      { role: "assistant", content: "第二轮回答", sources: [{}], graph: { nodes: [{}], edges: [] }, knowledgeGapRadar: { gaps: [{}] }, nextBestSource: { recommended: { label: "来源" } } },
    ],
    ...overrides,
  };
}

function sourceSelectionPayload() {
  return {
    title: "问前资料选择",
    status: "ready",
    summary: "建议先用 2 条最相关来源回答。",
    recommended: [
      {
        author: "飞翔的波波",
        date: "2025-08-08",
        title: "新手必看！亚马逊测款实战指南",
        excerpt: "点击率反映主图吸引力。",
        sourcePath: "飞翔的波波html/example.html",
        sourceCanUseAsEvidence: true,
        canUseAsEvidence: false,
      },
    ],
    alternatives: [],
    intent: {
      type: "method_learning",
      label: "方法学习",
      primaryAction: "先核对关键来源",
      boundary: "这一步只把作者原文作为资料证据。",
      confidence: "high",
    },
    intentChoices: [
      { type: "method_learning", label: "方法学习", primaryAction: "先核对关键来源", recommended: true },
      { type: "product_diagnosis", label: "产品诊断", primaryAction: "先看诊断优先级" },
    ],
    sourceControls: {
      excludedSourceKeys: [],
      allowedAuthors: ["飞翔的波波"],
      allowedSourceKeys: ["飞翔的波波html/example.html"],
      selectedSources: [{ author: "飞翔的波波", title: "新手必看！亚马逊测款实战指南" }],
    },
    boundary: "问前资料选择只决定下一轮先读哪些来源；候选理由是系统整理，不是新的作者原文证据，也不会写入知识库。",
  };
}

function studyPackPayload() {
  return {
    title: "主图视觉点击率转化率怎么优化？",
    boundary: "本地学习包预览是系统整理，不是作者原文证据；不包含音频或视频功能。",
    takeaways: [{ label: "主图", text: "先看点击入口。", identity: "来源支撑的系统整理", support: "source_backed" }],
    checklist: [{ label: "核对证据", reason: "先看原文。" }],
    reviewQuestions: [{ question: "核心判断是什么？", answer: "先看点击入口。" }],
    concepts: [{ label: "主图" }],
    sourceLedger: [{ title: "新手必看！亚马逊测款实战指南", author: "飞翔的波波", identity: "作者原文证据来源" }],
    markdown: "# 主图视觉点击率转化率怎么优化？\n\n## 来源账本\n- 飞翔的波波 · 新手必看！亚马逊测款实战指南\n",
    exportMarkdown: "# 主图视觉点击率转化率怎么优化？\n\n## 来源账本\n- 飞翔的波波 · 新手必看！亚马逊测款实战指南\n\n## 本地学习包预览\n\n### 系统掌握度自测（非作者证据）\n",
    studio: {
      reportSections: [{ title: "核心结论", items: ["先看点击入口。"] }],
      flashcards: [{ front: "核心判断是什么？", back: "先看点击入口。" }],
      mindMap: { nodes: [{ id: "topic", type: "topic", label: "主图" }], edges: [] },
      sourceTable: [{ title: "新手必看！亚马逊测款实战指南" }],
      masteryQuiz: {
        title: "掌握度自测",
        boundary: "自测只检查你对本专题的理解，不会写入作者原文证据，也不会自动保存为学习结论。",
        scoring: "点“掌握”或“再练”只保存在当前浏览器。",
        items: [{ id: "quiz-1", question: "核心判断是什么？", expectedAnswer: "先看点击入口。", sourceIndexes: [0] }],
      },
      actionPlan: {
        title: "亚马逊行动实验计划",
        boundary: "行动实验计划不会写入作者原文证据。",
        summary: "先核对来源，再补业务数据。",
        steps: [{ id: "action-1", label: "核对主图", requiredData: ["CTR"], successSignal: "CTR 改善", sourceIndexes: [0] }],
      },
      flashcardsCsv: "\"正面\",\"背面\"\n\"核心判断是什么？\",\"先看点击入口。\"\n",
      sourceTableCsv: "\"作者\",\"标题\"\n\"飞翔的波波\",\"新手必看！亚马逊测款实战指南\"\n",
    },
  };
}

function topicSwitchPayload() {
  return {
    sessionId: "topic-switch-session",
    firstSourceSelection: sourceSelectionPayload(),
    first: askPayload(TOPIC_SWITCH_ACCEPTANCE_SCENARIO.firstQuestion),
    standaloneResults: [
      {
        id: "persona",
        response: askPayload("人群画像应该怎么构建？有哪些实操指导建议", {
          answer: "问题：人群画像应该怎么构建？有哪些实操指导建议\n\n这次没有从本地知识库里找到足够相关的资料。 【缺少来源】",
          sources: [],
          sourceScope: { summary: "本轮使用全部作者资料。" },
        }),
      },
      {
        id: "selection-methods",
        response: askPayload("列出所有选品实操的可落地执行方法？", {
          answer: "问题：列出所有选品实操的可落地执行方法？\n\n可执行结论：先做市场需求、竞争、利润和产品差异化判断。",
          sourceScope: { summary: "本轮使用全部作者资料。" },
        }),
      },
    ],
  };
}

test("validateAmazonQaSmoke accepts a source-backed two-turn answer path", () => {
  const report = validateAmazonQaSmoke({
    status: statusPayload(),
    sourceSelection: sourceSelectionPayload(),
    first: askPayload("主图视觉点击率转化率怎么优化？"),
    second: askPayload("那我应该先改哪一块？"),
    sourceContext: { status: "located", match: "点击率反映主图吸引力" },
    studyPack: studyPackPayload(),
  });

  assert.equal(report.ok, true);
  assert.equal(report.documents, 1779);
  assert.equal(report.chunks, 14597);
  assert.equal(report.embeddedChunks, 14597);
  assert.equal(report.firstSources, 1);
  assert.equal(report.sourceSelectionStatus, "ready");
  assert.equal(report.secondGraphNodes, 2);
  assert.equal(report.secondNextBestSourceKind, "source");
  assert.equal(report.studioMasteryItems, 1);
  assert.equal(report.studioActionSteps, 1);
  assert.equal(report.sourceContextStatus, "located");
});

test("validateAmazonQaFinalAcceptance accepts three realistic Amazon learning questions", () => {
  const scenarios = FINAL_ACCEPTANCE_SCENARIOS.map((scenario) => ({
    id: scenario.id,
    first: askPayload(scenario.question),
    second: scenario.followUp ? askPayload(scenario.followUp) : undefined,
  }));
  const report = validateAmazonQaFinalAcceptance({
    status: statusPayload(),
    sourceSelections: FINAL_ACCEPTANCE_SCENARIOS.map(() => sourceSelectionPayload()),
    scenarios,
    sourceContext: { status: "located", match: "点击率反映主图吸引力" },
    notebook: {
      id: "acceptance-notebook",
      boundary: "学习专题会话不是作者原文证据。",
      messages: [
        { role: "user", content: FINAL_ACCEPTANCE_SCENARIOS[0].question },
        { role: "assistant", content: "第一轮" },
        { role: "user", content: FINAL_ACCEPTANCE_SCENARIOS[0].followUp },
        { role: "assistant", content: "第二轮" },
      ],
    },
    studyPack: studyPackPayload(),
    topicSwitch: topicSwitchPayload(),
  });

  assert.equal(report.ok, true);
  assert.equal(report.scenarios.length, 3);
  assert.deepEqual(report.scenarios.map((item) => item.id), ["visual-conversion", "product-selection", "listing-keywords"]);
  assert.equal(report.scenarios[0].followUp, "那我应该先改哪一块？");
  assert.ok(report.scenarios.every((item) => item.sources >= 1 && item.graphNodes >= 1 && item.learningQueueItems >= 1));
  assert.equal(report.topicSwitch.standaloneResults.length, 2);
  assert.deepEqual(report.topicSwitch.standaloneResults.map((item) => item.id), ["persona", "selection-methods"]);
  assert.equal(report.topicSwitch.standaloneResults[0].sources, 0);
  assert.equal(report.topicSwitch.standaloneResults[1].sources, 1);
});

test("validateAmazonQaSmoke rejects incomplete semantic index status", () => {
  assert.throws(
    () => validateAmazonQaSmoke({
      status: statusPayload({
        health: {
          documents: 1779,
          chunks: 14597,
          embeddedChunks: 12000,
          vectorCoveragePercent: 82.2,
          sourceTree: { ingestedDocuments: 1779, manifestDocuments: 1779, failedJobs: 0 },
        },
      }),
      first: askPayload("主图视觉点击率转化率怎么优化？"),
      second: askPayload("那我应该先改哪一块？"),
      studyPack: studyPackPayload(),
    }),
    /semantic index/i,
  );
});

test("validateAmazonQaSmoke rejects a follow-up without graph and source-backed study artifacts", () => {
  assert.throws(
    () => validateAmazonQaSmoke({
      status: statusPayload(),
      first: askPayload("主图视觉点击率转化率怎么优化？"),
      second: askPayload("那我应该先改哪一块？", {
        sources: [],
        graph: { nodes: [], edges: [] },
        notebookGuide: undefined,
        synthesisAnswer: undefined,
      }),
    }),
    /follow-up/i,
  );
});
