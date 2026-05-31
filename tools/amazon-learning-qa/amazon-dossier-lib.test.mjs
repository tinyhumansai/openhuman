import test from "node:test";
import assert from "node:assert/strict";

import {
  buildDossierSessionSeed,
  buildDossierOverview,
  buildDossierWorkbench,
  buildOpenHumanMemoryDocument,
  buildProductIntake,
  buildLearningDossier,
  normalizeStoredDossier,
  validateLearningDossierForSave,
  updateDossierBusinessVerificationState,
  updateDossierEvidenceDecisionState,
  updateDossierExperimentResultState,
  updateDossierReviewState,
  updateDossierSelfTestState,
} from "./amazon-dossier-lib.mjs";

const SAMPLE_VALIDATION_PACK = {
  title: "本轮业务验证任务包",
  status: "source_backed",
  summary: "把本轮作者证据转成真实业务数据检查，避免只停留在资料总结。",
  boundary: "任务包里的假设来自本轮作者原文证据；用户回填的数据只用于复核，不会变成作者原文证据。",
  hypotheses: [
    {
      id: "hypothesis:0",
      label: "产品首图极大程度上决定了点击率。",
      sourceIndex: 0,
      author: "跨境电商长期主义",
      sourceTitle: "你是如何解决转化率的？",
      quote: "产品首图极大程度上决定了点击率。",
      verifyWith: "用真实 CTR、CVR 和主图对比验证。",
    },
  ],
  dataRequests: [
    {
      id: "ctr",
      label: "当前点击率 CTR",
      why: "判断问题是否先发生在搜索结果点击入口。",
      placeholder: "例如：核心词 CTR 0.25%，曝光 12000，点击 30",
    },
    {
      id: "image",
      label: "主图/副图截图",
      why: "确认主图差异化和副图解释力。",
    },
  ],
  experiments: [
    {
      id: "visual-split",
      title: "主图点击入口小实验",
      steps: ["保留价格和广告不变", "只替换主图差异化表达", "观察 7 天 CTR、CVR 和 ACOS"],
      successSignal: "CTR 明显上升且 CVR 不下降，说明主图入口优先级成立。",
    },
  ],
  decisionRules: [
    {
      if: "CTR 上升但转化率 CVR 不动",
      then: "主图入口可能改善了，下一步转向副图、五点、评价和价格承接。",
    },
  ],
  followUpPrompt: "我补充了验证数据，请结合本轮作者来源和这些数据判断下一步：\n主题：主图/视觉\n需要核对：当前点击率 CTR、主图/副图截图",
};

const SAMPLE_SYNTHESIS_ANSWER = {
  title: "本轮综合答案",
  status: "source_backed",
  summary: "本轮围绕主图和点击率综合了 1 条作者原文证据，覆盖 1 个来源。",
  sourceCoverage: {
    sourceCount: 1,
    evidenceCount: 1,
    authorCount: 1,
    authors: ["跨境电商长期主义"],
  },
  sourceClaimIds: ["source-evidence:0"],
  points: [
    {
      id: "synthesis-point:0",
      label: "主图：先处理点击入口",
      text: "围绕「主图」，本轮来源提示先把搜索结果里的视觉点击入口单独检查。",
      identity: "系统综合",
      evidenceKind: "system_synthesis",
      canUseAsEvidence: false,
      isInference: true,
      confidence: "medium",
      basis: "由本轮已定位的作者原文证据综合得到；表达本身不是作者原话。",
      claimIds: ["source-evidence:0"],
      sourceIndexes: [0],
      support: [
        {
          claimId: "source-evidence:0",
          sourceIndex: 0,
          identity: "作者原文",
          evidenceKind: "source_evidence",
          author: "跨境电商长期主义",
          title: "你是如何解决转化率的？",
          date: "2022-06-18",
          quote: "产品首图极大程度上决定了点击率。",
        },
      ],
    },
  ],
  authorPerspectives: [
    {
      author: "跨境电商长期主义",
      summary: "主张先看主图带来的点击入口，再继续检查页面承接。",
      sourceIndexes: [0],
      claimIds: ["source-evidence:0"],
    },
  ],
  conflicts: [],
  gaps: [
    {
      id: "synthesis-gap:author",
      label: "作者视角不足",
      reason: "当前只有 1 位作者来源，需要继续查其他作者是否有不同判断。",
    },
  ],
  boundary: "系统综合，不是新的作者原文证据；只有绑定的作者摘录可作为来源支撑。",
};

const SAMPLE_MESSAGE = {
  role: "assistant",
  createdAt: "2026-05-25T10:00:00.000Z",
  content: [
    "问题：主图视觉点击率转化率怎么优化？",
    "",
    "可执行结论：",
    "1. 先把主图当成点击率入口处理。 【推断1】",
  ].join("\n"),
  sources: [
    {
      author: "跨境电商长期主义",
      date: "2022-06-18",
      title: "你是如何解决转化率的？",
      sourceUrl: "https://mp.weixin.qq.com/s/example",
      sourcePath: "跨境电商长期主义html/example.html",
      excerpt: "产品首图极大程度上决定了点击率。大家都在提图片优化，价格优势，产品评价和页面文案。",
    },
  ],
  learningCard: {
    intent: { type: "visual", label: "视觉转化学习档案" },
    takeaway: "先处理主图点击率，再看页面转化。",
    conclusions: ["主图是点击率入口"],
    nextActions: ["检查主图差异化", "检查副图对比"],
    missingInputs: ["具体产品链接"],
    followUps: ["我的主图应该怎么改？"],
  },
  evidenceFeedback: {
    "source-evidence:0": "useful",
    "source-evidence:1": "irrelevant",
  },
  evidenceChain: {
    claims: [
      {
        id: "source-evidence:0",
        type: "source_evidence",
        text: "产品首图决定点击率",
        quote: "产品首图极大程度上决定了点击率。",
        sourceIndex: 0,
        author: "跨境电商长期主义",
        title: "你是如何解决转化率的？",
        date: "2022-06-18",
      },
      {
        id: "source-evidence:1",
        type: "source_evidence",
        text: "泛泛而谈图片优化",
        quote: "大家都在提图片优化，价格优势，产品评价和页面文案。",
        sourceIndex: 0,
        author: "跨境电商长期主义",
        title: "你是如何解决转化率的？",
        date: "2022-06-18",
      },
      {
        id: "system-inference:0",
        type: "system_inference",
        text: "先处理主图。",
      },
    ],
  },
  validationPack: SAMPLE_VALIDATION_PACK,
  synthesisAnswer: SAMPLE_SYNTHESIS_ANSWER,
};

const PRODUCT_DIAGNOSIS_MESSAGE = {
  ...SAMPLE_MESSAGE,
  productInputSummary: {
    source: "user_input",
    summary: "本轮识别到 3 类用户产品信息：主图/视觉、点击率/转化率数据、关键词/竞品。",
    facts: [
      {
        id: "visual",
        label: "主图/视觉",
        items: ["主图是白底图，和竞品差不多"],
        missing: ["竞品主图对照"],
      },
      {
        id: "metrics",
        label: "点击率/转化率数据",
        items: ["CTR 0.25%，CVR 5%，最近 7 天 session 1200"],
        missing: ["流量来源"],
      },
      {
        id: "keywords",
        label: "关键词/竞品",
        items: ["核心关键词 garlic press，竞品 ASIN B001234567"],
        missing: [],
      },
    ],
    missing: ["广告 ACOS", "产品链接"],
    caution: "这些是用户提供的诊断输入，不是本地资料证据。",
  },
  diagnosisPanel: {
    summary: "把本轮诊断拆成可执行排查项，勾选状态只保存在当前对话。",
    priority: "先改搜索结果里的主图点击入口",
    reason: "CTR 低于正常入口水平，先检查主图和关键词匹配。",
    tracks: [
      {
        id: "visual",
        label: "主图入口检查",
        level: "优先",
        why: "主图直接影响点击入口。",
        prompt: "请继续诊断我的主图入口。",
        checks: [
          { id: "competitor", label: "对比前三名竞品主图" },
          { id: "claim", label: "确认主图是否表达核心卖点" },
        ],
      },
      {
        id: "listing",
        label: "Listing 承接检查",
        level: "随后",
        why: "点击后仍要看页面是否承接。",
        prompt: "请继续诊断我的 Listing 承接。",
        checks: [{ id: "title", label: "检查标题关键词是否承接主图卖点" }],
      },
    ],
    checked: {
      "visual::competitor": true,
    },
    caution: "诊断勾选代表用户排查进度，不是原文证据。",
  },
};

test("buildLearningDossier preserves accepted and rejected evidence with source identity", () => {
  const dossier = buildLearningDossier({
    id: "dossier-test",
    createdAt: "2026-05-25T10:10:00.000Z",
    question: "主图视觉点击率转化率怎么优化？",
    message: SAMPLE_MESSAGE,
    sourceControls: {
      excludedSourceKeys: ["跨境电商长期主义html/blocked-example.html"],
      allowedAuthors: ["跨境电商长期主义"],
    },
  });

  assert.equal(dossier.id, "dossier-test");
  assert.equal(dossier.title, "视觉转化学习档案");
  assert.equal(dossier.question, "主图视觉点击率转化率怎么优化？");
  assert.deepEqual(dossier.conclusions, ["主图是点击率入口"]);
  assert.deepEqual(dossier.nextActions.slice(0, 2), ["检查主图差异化", "检查副图对比"]);
  assert.equal(dossier.acceptedEvidence.length, 1);
  assert.equal(dossier.rejectedEvidence.length, 1);
  assert.equal(dossier.acceptedEvidence[0].sourcePath, "跨境电商长期主义html/example.html");
  assert.equal(dossier.acceptedEvidence[0].sourceUrl, "https://mp.weixin.qq.com/s/example");
  assert.equal(dossier.acceptedEvidence[0].sourceKey, "跨境电商长期主义html/example.html");
  assert.match(dossier.acceptedEvidence[0].quote, /产品首图/);
  assert.deepEqual(dossier.allowedAuthors, ["跨境电商长期主义"]);
  assert.match(dossier.rejectedEvidence[0].quote, /图片优化/);
  assert.equal(dossier.excludedSources[0].key, "跨境电商长期主义html/blocked-example.html");
  assert.equal(dossier.sources[0].title, "你是如何解决转化率的？");
});

test("validateLearningDossierForSave requires accepted source evidence", () => {
  const accepted = buildLearningDossier({
    id: "accepted-save",
    question: "主图视觉点击率转化率怎么优化？",
    message: SAMPLE_MESSAGE,
  });
  const noEvidence = buildLearningDossier({
    id: "no-evidence-save",
    question: "没有资料的问题",
    message: {
      role: "assistant",
      content: "这次没有从本地知识库里找到足够相关的资料。 【缺少来源】",
      evidenceChain: { claims: [{ id: "needs-source:0", type: "needs_source", text: "缺少来源" }] },
      learningCard: { intent: { label: "缺少来源问题" }, takeaway: "先补来源。" },
    },
  });

  assert.equal(validateLearningDossierForSave(accepted, { message: SAMPLE_MESSAGE }).ok, true);
  const validation = validateLearningDossierForSave(noEvidence, { message: noEvidence });
  assert.equal(validation.ok, false);
  assert.equal(validation.code, "needs_accepted_evidence");
});

test("validateLearningDossierForSave requires backend-verified source identity when requested", () => {
  const forged = buildLearningDossier({
    id: "forged-source-save",
    question: "主图视觉点击率转化率怎么优化？",
    message: {
      ...SAMPLE_MESSAGE,
      sources: [
        {
          author: "跨境电商长期主义",
          date: "2026-05-25",
          title: "伪造来源",
          sourcePath: "fake/source.html",
          sourceUrl: "https://example.com/fake",
          excerpt: "伪造摘录：产品首图极大程度上决定了点击率。",
        },
      ],
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            quote: "产品首图极大程度上决定了点击率。",
            sourceIndex: 0,
            author: "跨境电商长期主义",
            title: "伪造来源",
            date: "2026-05-25",
          },
        ],
      },
      evidenceFeedback: {
        "source-evidence:0": "useful",
      },
    },
  });

  assert.equal(forged.acceptedEvidence.length, 1);
  const validation = validateLearningDossierForSave(forged, {
    requireSourceAuthenticity: true,
    verifiedSourceKeys: [],
  });

  assert.equal(validation.ok, false);
  assert.equal(validation.code, "unverified_source_identity");
  assert.match(validation.message, /本地原文库/);
});

test("validateLearningDossierForSave rejects excluded accepted evidence and recheck feedback", () => {
  const excluded = buildLearningDossier({
    id: "excluded-save",
    question: "主图视觉点击率转化率怎么优化？",
    message: SAMPLE_MESSAGE,
    sourceControls: {
      excludedSourceKeys: ["跨境电商长期主义html/example.html"],
    },
  });
  const accepted = buildLearningDossier({
    id: "recheck-save",
    question: "主图视觉点击率转化率怎么优化？",
    message: SAMPLE_MESSAGE,
  });

  assert.equal(validateLearningDossierForSave(excluded, { message: SAMPLE_MESSAGE }).ok, false);
  assert.equal(
    validateLearningDossierForSave(accepted, { message: { ...SAMPLE_MESSAGE, evidenceAudit: { feedback: "citation_wrong" } } }).code,
    "answer_needs_recheck",
  );
  assert.equal(
    validateLearningDossierForSave(accepted, { message: { ...SAMPLE_MESSAGE, evidenceAudit: { feedback: "retry" } } }).ok,
    false,
  );
});

test("updateDossierEvidenceDecisionState promotes stored source excerpts only by user decision", () => {
  const dossier = buildLearningDossier({
    id: "source-decision",
    question: "主图怎么优化？",
    message: {
      ...SAMPLE_MESSAGE,
      evidenceFeedback: {},
    },
  });

  assert.equal(dossier.acceptedEvidence.length, 0);
  assert.equal(dossier.sources.length, 1);

  const accepted = updateDossierEvidenceDecisionState(dossier, {
    sourceIndex: 0,
    decision: "useful",
  });

  assert.equal(accepted.acceptedEvidence.length, 1);
  assert.equal(accepted.rejectedEvidence.length, 0);
  assert.equal(accepted.acceptedEvidence[0].sourceKey, "跨境电商长期主义html/example.html");
  assert.match(accepted.acceptedEvidence[0].quote, /产品首图极大程度上决定了点击率/);
  assert.ok(!accepted.acceptedEvidence[0].quote.includes("任务包"));

  const rejected = updateDossierEvidenceDecisionState(accepted, {
    sourceIndex: 0,
    decision: "irrelevant",
  });

  assert.equal(rejected.acceptedEvidence.length, 0);
  assert.equal(rejected.rejectedEvidence.length, 1);
  assert.equal(rejected.rejectedEvidence[0].sourceKey, "跨境电商长期主义html/example.html");
});

test("buildLearningDossier preserves validation pack without promoting it to evidence", () => {
  const dossier = buildLearningDossier({
    id: "validation-pack-dossier",
    createdAt: "2026-05-25T10:10:00.000Z",
    question: "主图视觉点击率转化率怎么优化？",
    message: SAMPLE_MESSAGE,
  });

  assert.equal(dossier.validationPack.status, "source_backed");
  assert.match(dossier.validationPack.boundary, /不会变成作者原文证据/);
  assert.equal(dossier.validationPack.hypotheses.length, 1);
  assert.equal(dossier.validationPack.hypotheses[0].sourceIndex, 0);
  assert.equal(dossier.validationPack.dataRequests[0].id, "ctr");
  assert.equal(dossier.validationPack.experiments[0].id, "visual-split");
  assert.match(dossier.validationPack.followUpPrompt, /验证数据/);
  assert.equal(dossier.acceptedEvidence.length, 1);
  assert.ok(dossier.acceptedEvidence.every((item) => item.sourceKey));
  assert.ok(!dossier.acceptedEvidence.some((item) => String(item.quote || item.text).includes("任务包")));
});

test("buildLearningDossier preserves synthesis answer without promoting it to evidence", () => {
  const dossier = buildLearningDossier({
    id: "synthesis-dossier",
    createdAt: "2026-05-25T10:10:00.000Z",
    question: "主图视觉点击率转化率怎么优化？",
    message: SAMPLE_MESSAGE,
  });

  assert.equal(dossier.synthesisAnswer.status, "source_backed");
  assert.equal(dossier.synthesisAnswer.points.length, 1);
  assert.equal(dossier.synthesisAnswer.points[0].evidenceKind, "system_synthesis");
  assert.equal(dossier.synthesisAnswer.points[0].canUseAsEvidence, false);
  assert.equal(dossier.synthesisAnswer.points[0].support.length, 1);
  assert.equal(dossier.synthesisAnswer.points[0].support[0].sourceIndex, 0);
  assert.equal(dossier.synthesisAnswer.sourceCoverage.evidenceCount, 1);
  assert.equal(dossier.acceptedEvidence.length, 1);
  assert.ok(!dossier.acceptedEvidence.some((item) => String(item.quote || item.text).includes("视觉点击入口单独检查")));
});

test("normalizeStoredDossier trims synthesis support to existing dossier sources", () => {
  const dossier = normalizeStoredDossier({
    id: "stored-synthesis",
    sources: SAMPLE_MESSAGE.sources,
    acceptedEvidence: [
      {
        quote: "产品首图极大程度上决定了点击率。",
        text: "产品首图决定点击率",
        author: "跨境电商长期主义",
        title: "你是如何解决转化率的？",
        sourcePath: "跨境电商长期主义html/example.html",
        sourceKey: "跨境电商长期主义html/example.html",
      },
    ],
    synthesisAnswer: {
      ...SAMPLE_SYNTHESIS_ANSWER,
      status: "source_backed",
      sourceCoverage: {
        sourceCount: 99,
        evidenceCount: 99,
        authorCount: 99,
        authors: ["跨境电商长期主义", "伪造作者"],
      },
      points: [
        {
          ...SAMPLE_SYNTHESIS_ANSWER.points[0],
          support: [
            SAMPLE_SYNTHESIS_ANSWER.points[0].support[0],
            {
              claimId: "fake",
              sourceIndex: 9,
              author: "用户输入",
              title: "伪造来源",
              quote: "CTR 0.25%，ACOS 45%",
            },
          ],
        },
      ],
    },
  });

  assert.equal(dossier.synthesisAnswer.points.length, 1);
  assert.equal(dossier.synthesisAnswer.points[0].support.length, 1);
  assert.equal(dossier.synthesisAnswer.points[0].support[0].sourceIndex, 0);
  assert.equal(dossier.synthesisAnswer.status, "source_backed");
  assert.equal(dossier.synthesisAnswer.sourceCoverage.sourceCount, 1);
  assert.equal(dossier.synthesisAnswer.sourceCoverage.evidenceCount, 1);
  assert.equal(dossier.synthesisAnswer.sourceCoverage.authorCount, 1);
  assert.deepEqual(dossier.synthesisAnswer.sourceCoverage.authors, ["跨境电商长期主义"]);
  assert.equal(dossier.acceptedEvidence.length, 1);
  assert.ok(!dossier.acceptedEvidence.some((item) => String(item.quote || item.text).includes("CTR 0.25%")));
});

test("normalizeStoredDossier downgrades synthesis status when stored support is invalid", () => {
  const dossier = normalizeStoredDossier({
    id: "invalid-synthesis-support",
    sources: SAMPLE_MESSAGE.sources,
    synthesisAnswer: {
      title: "本轮综合答案",
      status: "source_backed",
      summary: "伪造状态不应保留为来源支撑。",
      sourceCoverage: {
        sourceCount: 12,
        evidenceCount: 12,
        authorCount: 12,
        authors: ["伪造作者"],
      },
      points: [
        {
          id: "point",
          label: "伪造支撑",
          text: "这条综合要点没有有效来源。",
          claimIds: ["fake"],
          sourceIndexes: [0],
          support: [{ sourceIndex: 9, author: "伪造作者", title: "伪造来源", quote: "伪造摘录" }],
        },
      ],
    },
  });

  assert.equal(dossier.synthesisAnswer.status, "needs_source");
  assert.equal(dossier.synthesisAnswer.sourceCoverage.sourceCount, 0);
  assert.equal(dossier.synthesisAnswer.sourceCoverage.evidenceCount, 0);
  assert.equal(dossier.synthesisAnswer.sourceCoverage.authorCount, 0);
  assert.deepEqual(dossier.synthesisAnswer.sourceCoverage.authors, []);
  assert.deepEqual(dossier.synthesisAnswer.sourceClaimIds, []);
  assert.deepEqual(dossier.synthesisAnswer.points[0].claimIds, []);
  assert.deepEqual(dossier.synthesisAnswer.points[0].sourceIndexes, []);
  assert.equal(dossier.synthesisAnswer.points[0].support.length, 0);
});

test("normalizeStoredDossier creates a conservative synthesis guide for old dossiers", () => {
  const dossier = normalizeStoredDossier({
    id: "legacy-dossier",
    title: "视觉转化诊断",
    question: "主图视觉点击率转化率怎么优化？",
    takeaway: "先把主图当成点击入口处理。",
    conclusions: ["主图是点击率入口"],
    nextActions: ["检查主图差异化"],
  });

  assert.equal(dossier.synthesisAnswer.status, "needs_source");
  assert.match(dossier.synthesisAnswer.summary, /旧学习档案/);
  assert.match(dossier.synthesisAnswer.boundary, /系统综合不是作者原文证据/);
  assert.equal(dossier.synthesisAnswer.points.length, 3);
  assert.equal(dossier.synthesisAnswer.points[0].canUseAsEvidence, false);
  assert.equal(dossier.synthesisAnswer.points[0].support.length, 0);
  assert.equal(dossier.acceptedEvidence.length, 0);
});

test("normalizeStoredDossier backs legacy synthesis only with matching accepted source evidence", () => {
  const dossier = normalizeStoredDossier({
    id: "legacy-source-backed",
    title: "视觉转化诊断",
    question: "主图视觉点击率转化率怎么优化？",
    takeaway: "先把主图当成点击入口处理。",
    sources: SAMPLE_MESSAGE.sources,
    acceptedEvidence: [
      {
        quote: "产品首图极大程度上决定了点击率。",
        text: "产品首图决定点击率",
        author: "跨境电商长期主义",
        title: "你是如何解决转化率的？",
        sourcePath: "跨境电商长期主义html/example.html",
        sourceKey: "跨境电商长期主义html/example.html",
      },
    ],
  });

  assert.equal(dossier.synthesisAnswer.status, "source_backed");
  assert.equal(dossier.synthesisAnswer.sourceCoverage.sourceCount, 1);
  assert.equal(dossier.synthesisAnswer.points[0].support.length, 1);
  assert.equal(dossier.synthesisAnswer.points[0].support[0].sourceIndex, 0);
  assert.equal(dossier.acceptedEvidence.length, 1);
});

test("buildLearningDossier preserves workflow intent through saved dossier and session restore", () => {
  const dossier = normalizeStoredDossier(buildLearningDossier({
    id: "workflow-intent",
    question: "我这个 Listing 该先改哪？",
    message: {
      ...SAMPLE_MESSAGE,
      workflowIntent: {
        type: "product_diagnosis",
        label: "产品诊断",
        goal: "用用户产品事实对照作者方法。",
        primaryAction: "先看诊断优先级。",
        nextPrompt: "请继续诊断我的 Listing。",
        boundary: "用户产品材料不是作者原文证据。",
        confidence: "medium",
      },
    },
  }));
  const seed = buildDossierSessionSeed(dossier);

  assert.equal(dossier.workflowIntent.type, "product_diagnosis");
  assert.equal(dossier.title, "产品诊断");
  assert.equal(seed.messages[1].workflowIntent.type, "product_diagnosis");
  assert.match(seed.messages[1].workflowIntent.boundary, /不是作者原文证据/);
});

test("buildOpenHumanMemoryDocument turns a dossier into a separate workflow memory doc", () => {
  const dossier = updateDossierExperimentResultState(
    updateDossierBusinessVerificationState(
      buildLearningDossier({
        id: "memory-doc",
        createdAt: "2026-05-25T10:00:00.000Z",
        question: "主图视觉点击率转化率怎么优化？",
        message: PRODUCT_DIAGNOSIS_MESSAGE,
      }),
      {
        text: "主图是白底图，CTR 0.25%，CVR 5%，核心关键词 garlic press",
        createdAt: "2026-05-25T10:30:00.000Z",
      },
    ),
    {
      text: "主图 A/B 测试 7 天，CTR 从 0.25% 到 0.42%，CVR 从 5% 到 5.2%，ACOS 从 45% 到 38%",
      createdAt: "2026-05-25T11:30:00.000Z",
    },
  );
  const doc = buildOpenHumanMemoryDocument(dossier, { sourceNamespace: "amazon-learning" });

  assert.equal(doc.namespace, "amazon-learning-workflow");
  assert.equal(doc.key, "dossier/memory-doc");
  assert.equal(doc.source_type, "amazon-learning-dossier");
  assert.equal(doc.priority, "high");
  assert.ok(doc.tags.includes("openhuman-evidence-workflow"));
  assert.match(doc.title, /亚马逊学习档案/);
  assert.match(doc.content, /已采纳原文证据/);
  assert.match(doc.content, /用户业务材料/);
  assert.match(doc.content, /实验复盘/);
  assert.match(doc.content, /业务验证任务包/);
  assert.match(doc.content, /任务包不是作者原文证据/);
  assert.match(doc.content, /本轮综合讲义/);
  assert.match(doc.content, /视觉点击入口单独检查/);
  assert.match(doc.content, /系统综合不是作者原文证据/);
  assert.match(doc.content, /当前点击率 CTR/);
  assert.match(doc.content, /主图点击入口小实验/);
  assert.match(doc.content, /用户业务材料不是作者原文证据/);
  assert.equal(doc.metadata.dossier_id, "memory-doc");
  assert.equal(doc.metadata.source_namespace, "amazon-learning");
  assert.equal(doc.metadata.accepted_evidence_count, 1);
  assert.equal(doc.metadata.business_verification_records, 1);
  assert.equal(doc.metadata.experiment_results, 1);
  assert.equal(doc.metadata.validation_pack_status, "source_backed");
  assert.equal(doc.metadata.synthesis_status, "source_backed");
});

test("buildOpenHumanMemoryDocument does not store no-source conclusions as reusable knowledge", () => {
  const dossier = buildLearningDossier({
    id: "memory-doc-no-source",
    createdAt: "2026-05-25T10:00:00.000Z",
    question: "没有资料的问题",
    message: {
      role: "assistant",
      content: "这次没有从本地知识库里找到足够相关的资料。 【缺少来源】",
      evidenceChain: { claims: [{ id: "needs-source:0", type: "needs_source", text: "缺少来源" }] },
      learningCard: {
        intent: { label: "缺少来源问题" },
        takeaway: "先补具体产品数据。",
        conclusions: ["先补具体产品数据。"],
        nextActions: ["重新检索作者资料"],
        missingInputs: ["产品链接", "关键词"],
      },
    },
  });
  const doc = buildOpenHumanMemoryDocument(dossier, { sourceNamespace: "amazon-learning" });

  assert.doesNotMatch(doc.content, /## 可复用结论/);
  assert.doesNotMatch(doc.content, /## 有来源支撑的结论[\s\S]{0,120}先补具体产品数据/);
  assert.match(doc.content, /证据状态：暂无已采纳原文证据/);
  assert.match(doc.content, /## 待验证理解/);
  assert.match(doc.content, /先补具体产品数据/);
});

test("normalizeStoredDossier preserves OpenHuman memory sync status", () => {
  const dossier = normalizeStoredDossier({
    id: "memory-status",
    openhumanMemory: {
      namespace: "amazon-learning-workflow",
      key: "dossier/memory-status",
      documentId: "doc_123",
      status: "synced",
      syncedAt: "2026-05-25T11:00:00.000Z",
      indexStatus: "missing_embeddings",
      totalChunks: 7,
      indexedChunks: 0,
    },
  });

  assert.equal(dossier.openhumanMemory.namespace, "amazon-learning-workflow");
  assert.equal(dossier.openhumanMemory.key, "dossier/memory-status");
  assert.equal(dossier.openhumanMemory.documentId, "doc_123");
  assert.equal(dossier.openhumanMemory.status, "synced");
  assert.equal(dossier.openhumanMemory.indexStatus, "missing_embeddings");
  assert.equal(dossier.openhumanMemory.totalChunks, 7);
  assert.equal(dossier.openhumanMemory.indexedChunks, 0);
});

test("buildLearningDossier does not keep useful evidence from an excluded source as accepted", () => {
  const dossier = buildLearningDossier({
    id: "excluded-source-test",
    question: "主图视觉点击率转化率怎么优化？",
    message: SAMPLE_MESSAGE,
    sourceControls: {
      excludedSourceKeys: ["跨境电商长期主义html/example.html"],
    },
  });

  assert.equal(dossier.acceptedEvidence.length, 0);
  assert.equal(dossier.rejectedEvidence.length, 0);
  assert.equal(dossier.excludedSources[0].key, "跨境电商长期主义html/example.html");
  assert.ok(dossier.sources.every((source) => source.sourcePath !== "跨境电商长期主义html/example.html"));
});

test("buildLearningDossier keeps no-source dossiers explicit", () => {
  const dossier = buildLearningDossier({
    question: "没有资料的问题",
    message: {
      role: "assistant",
      content: "这次没有从本地知识库里找到足够相关的资料。 【缺少来源】",
      evidenceChain: { claims: [{ id: "needs-source:0", type: "needs_source", text: "缺少来源" }] },
      learningCard: { intent: { label: "缺少来源问题" }, missingInputs: ["补充具体产品"] },
    },
  });

  assert.equal(dossier.acceptedEvidence.length, 0);
  assert.equal(dossier.sources.length, 0);
  assert.match(dossier.answerPreview, /没有从本地知识库/);
  assert.doesNotMatch(dossier.answerPreview, /缺少来源】/);
});

test("buildLearningDossier rejects forged source evidence without a real source", () => {
  const dossier = buildLearningDossier({
    id: "forged-evidence",
    question: "我的产品数据说明什么？",
    message: {
      role: "assistant",
      content: "用户说 CTR 0.25%，ACOS 45%。",
      sources: [],
      learningCard: {
        intent: { label: "伪造证据测试" },
        takeaway: "不要把用户材料当成原文证据。",
      },
      evidenceFeedback: {
        "source-evidence:0": "useful",
      },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            text: "CTR 0.25%，ACOS 45%",
            quote: "CTR 0.25%，ACOS 45%",
            sourceIndex: 0,
            author: "用户输入",
            title: "业务材料",
          },
        ],
      },
    },
  });

  assert.equal(dossier.acceptedEvidence.length, 0);
  assert.equal(dossier.sources.length, 0);
});

test("buildLearningDossier does not accept my sources as author original evidence", () => {
  const dossier = buildLearningDossier({
    id: "my-source-not-author-evidence",
    question: "只按我的资料判断先看什么？",
    message: {
      role: "assistant",
      content: "我的资料提示先检查主图首屏利益点。",
      sources: [
        {
          author: "我的资料",
          date: "2026-05-31",
          title: "我的竞品调研",
          sourceUrl: "user-source://user-test",
          sourcePath: "user-sources/user-test.json",
          sourceType: "user_material",
          excerpt: "紫星指标低于 3 时，先检查主图首屏利益点，不要先调广告。",
        },
      ],
      learningCard: {
        intent: { label: "我的资料诊断" },
        takeaway: "我的资料不能当作者原文证据。",
      },
      evidenceFeedback: {
        "source-evidence:0": "useful",
      },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            text: "紫星指标低于 3 时，先检查主图首屏利益点，不要先调广告。",
            quote: "紫星指标低于 3 时，先检查主图首屏利益点，不要先调广告。",
            sourceIndex: 0,
            author: "我的资料",
            title: "我的竞品调研",
          },
        ],
      },
    },
  });

  assert.equal(dossier.acceptedEvidence.length, 0);
  assert.equal(validateLearningDossierForSave(dossier, { message: dossier }).code, "needs_accepted_evidence");
});

test("buildLearningDossier rejects forged source evidence with invalid source index", () => {
  const dossier = buildLearningDossier({
    id: "invalid-source-index",
    question: "主图怎么改？",
    message: {
      ...SAMPLE_MESSAGE,
      evidenceFeedback: {
        "source-evidence:999": "useful",
      },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:999",
            type: "source_evidence",
            text: "伪造材料",
            quote: "伪造材料",
            sourceIndex: 999,
            author: "伪造作者",
            title: "伪造来源",
          },
        ],
      },
    },
  });

  assert.equal(dossier.acceptedEvidence.length, 0);
  assert.equal(dossier.rejectedEvidence.length, 0);
  assert.equal(dossier.sources.length, 1);
});

test("buildLearningDossier rejects forged quote attached to a real source", () => {
  const dossier = buildLearningDossier({
    id: "forged-quote-real-source",
    question: "我的产品数据说明什么？",
    message: {
      ...SAMPLE_MESSAGE,
      content: "用户说 CTR 0.25%，ACOS 45%。",
      evidenceFeedback: {
        "source-evidence:fake": "useful",
      },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:fake",
            type: "source_evidence",
            text: "CTR 0.25%，ACOS 45%",
            quote: "CTR 0.25%，ACOS 45%",
            sourceIndex: 0,
            author: "跨境电商长期主义",
            title: "你是如何解决转化率的？",
          },
        ],
      },
    },
  });

  assert.equal(dossier.acceptedEvidence.length, 0);
  assert.equal(dossier.rejectedEvidence.length, 0);
  assert.equal(dossier.sources.length, 1);
});

test("normalizeStoredDossier trims untrusted stored data", () => {
  const dossier = normalizeStoredDossier({
    id: "../bad",
    title: "x".repeat(200),
    acceptedEvidence: [{ quote: "q", sourcePath: "p" }],
    rejectedEvidence: [{ text: "r" }],
    excludedSources: [{ key: "k", label: "l" }],
    sources: [{ title: "source", author: "a" }],
  });

  assert.ok(dossier.title.length <= 120);
  assert.equal(dossier.acceptedEvidence.length, 1);
  assert.equal(dossier.rejectedEvidence.length, 1);
  assert.equal(dossier.excludedSources[0].key, "k");
  assert.equal(dossier.sources[0].title, "source");
});

test("buildDossierSessionSeed restores useful evidence and excluded sources for follow-up retrieval", () => {
  const dossier = buildLearningDossier({
    id: "dossier-test",
    createdAt: "2026-05-25T10:10:00.000Z",
    question: "主图视觉点击率转化率怎么优化？",
    message: SAMPLE_MESSAGE,
    sourceControls: {
      excludedSourceKeys: ["跨境电商长期主义html/blocked-example.html"],
      allowedAuthors: ["跨境电商长期主义"],
    },
  });
  const seed = buildDossierSessionSeed(dossier);

  assert.equal(seed.sourceControls.excludedSourceKeys[0], "跨境电商长期主义html/blocked-example.html");
  assert.deepEqual(seed.sourceControls.allowedAuthors, ["跨境电商长期主义"]);
  assert.deepEqual(seed.messages[1].sourceScope.allowedAuthors, ["跨境电商长期主义"]);
  assert.equal(seed.messages.length, 2);
  assert.equal(seed.messages[0].role, "user");
  assert.equal(seed.messages[0].content, "主图视觉点击率转化率怎么优化？");
  assert.equal(seed.messages[1].role, "assistant");
  assert.equal(seed.messages[1].restoredFromDossierId, "dossier-test");
  assert.match(seed.messages[1].content, /已从学习档案恢复上下文/);
  assert.ok(seed.messages[1].evidenceChain.claims.some((claim) => claim.type === "source_evidence"));
  assert.ok(Object.values(seed.messages[1].evidenceFeedback).includes("useful"));
  assert.ok(Object.values(seed.messages[1].evidenceFeedback).includes("irrelevant"));
  const usefulClaimId = Object.entries(seed.messages[1].evidenceFeedback).find(([, value]) => value === "useful")[0];
  const usefulClaim = seed.messages[1].evidenceChain.claims.find((claim) => claim.id === usefulClaimId);
  assert.match(usefulClaim.quote, /产品首图/);
});

test("buildDossierSessionSeed keeps no-source dossiers reopenable", () => {
  const dossier = buildLearningDossier({
    id: "no-source",
    question: "没有资料的问题",
    message: {
      role: "assistant",
      content: "这次没有从本地知识库里找到足够相关的资料。 【缺少来源】",
      learningCard: { intent: { label: "缺少来源问题" }, missingInputs: ["补充具体产品"] },
    },
  });
  const seed = buildDossierSessionSeed(dossier);

  assert.equal(seed.messages.length, 2);
  assert.equal(seed.messages[1].evidenceChain.claims[0].type, "needs_source");
  assert.deepEqual(seed.sourceControls.excludedSourceKeys, []);
});

test("learning dossiers preserve product input and diagnosis panel snapshots", () => {
  const dossier = buildLearningDossier({
    id: "diagnosis-dossier",
    createdAt: "2026-05-25T10:20:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });

  assert.equal(dossier.productInputSummary.source, "user_input");
  assert.equal(dossier.productInputSummary.facts.length, 3);
  assert.match(dossier.productInputSummary.facts[1].items[0], /CTR 0.25%/);
  assert.equal(dossier.diagnosisPanel.priority, "先改搜索结果里的主图点击入口");
  assert.equal(dossier.diagnosisPanel.tracks.length, 2);
  assert.equal(dossier.diagnosisPanel.checked["visual::competitor"], true);
  assert.equal(dossier.acceptedEvidence.length, 1);
  assert.ok(dossier.acceptedEvidence.every((item) => !item.text.includes("CTR 0.25%")));
});

test("stored dossier normalization trims diagnosis snapshots without creating evidence", () => {
  const dossier = normalizeStoredDossier({
    id: "stored-diagnosis",
    productInputSummary: PRODUCT_DIAGNOSIS_MESSAGE.productInputSummary,
    diagnosisPanel: {
      ...PRODUCT_DIAGNOSIS_MESSAGE.diagnosisPanel,
      tracks: Array.from({ length: 10 }, (_, index) => ({
        id: `track-${index}`,
        label: `检查组 ${index}`,
        checks: Array.from({ length: 10 }, (__, checkIndex) => ({
          id: `check-${checkIndex}`,
          label: `检查项 ${checkIndex}`,
        })),
      })),
      checked: {
        "track-0::check-0": true,
        "track-8::check-8": true,
      },
    },
    evidenceChain: {
      claims: [{ type: "source_evidence", text: "不应该被读取" }],
    },
  });

  assert.equal(dossier.productInputSummary.facts.length, 3);
  assert.equal(dossier.diagnosisPanel.tracks.length, 6);
  assert.equal(dossier.diagnosisPanel.tracks[0].checks.length, 6);
  assert.equal(dossier.diagnosisPanel.checked["track-0::check-0"], true);
  assert.equal(dossier.diagnosisPanel.checked["track-8::check-8"], undefined);
  assert.equal(dossier.acceptedEvidence.length, 0);
  assert.equal(dossier.rejectedEvidence.length, 0);
});

test("dossier reopen restores diagnosis snapshots without adding source evidence", () => {
  const dossier = buildLearningDossier({
    id: "diagnosis-dossier",
    createdAt: "2026-05-25T10:20:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const seed = buildDossierSessionSeed(dossier);
  const assistant = seed.messages[1];

  assert.equal(assistant.productInputSummary.facts[0].label, "主图/视觉");
  assert.equal(assistant.diagnosisPanel.checked["visual::competitor"], true);
  assert.match(assistant.content, /已保存产品诊断/);
  assert.equal(assistant.evidenceChain.claims.filter((claim) => claim.type === "source_evidence").length, 2);
  assert.ok(assistant.evidenceChain.claims.every((claim) => !String(claim.text || "").includes("CTR 0.25%")));
});

test("dossier reopen restores synthesis answer as a learning snapshot only", () => {
  const dossier = buildLearningDossier({
    id: "synthesis-reopen",
    createdAt: "2026-05-25T10:20:00.000Z",
    question: "主图视觉点击率转化率怎么优化？",
    message: SAMPLE_MESSAGE,
  });
  const seed = buildDossierSessionSeed(dossier);
  const assistant = seed.messages[1];

  assert.equal(assistant.synthesisAnswer.status, "source_backed");
  assert.equal(assistant.synthesisAnswer.points.length, 1);
  assert.equal(assistant.synthesisAnswer.points[0].canUseAsEvidence, false);
  assert.equal(assistant.evidenceChain.claims.filter((claim) => claim.type === "source_evidence").length, 2);
  assert.ok(!assistant.evidenceChain.claims.some((claim) => String(claim.text || "").includes("视觉点击入口单独检查")));
});

test("dossier workbench exposes diagnosis progress as archive metadata", () => {
  const dossier = buildLearningDossier({
    id: "diagnosis-dossier",
    createdAt: "2026-05-25T10:20:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const workbench = buildDossierWorkbench(dossier);

  assert.equal(workbench.diagnosis.priority, "先改搜索结果里的主图点击入口");
  assert.equal(workbench.diagnosis.totalTracks, 2);
  assert.equal(workbench.diagnosis.totalChecks, 3);
  assert.equal(workbench.diagnosis.checkedChecks, 1);
});

test("dossier workbench exposes synthesis guide without changing evidence progress", () => {
  const dossier = buildLearningDossier({
    id: "synthesis-workbench",
    createdAt: "2026-05-25T10:20:00.000Z",
    question: "主图视觉点击率转化率怎么优化？",
    message: SAMPLE_MESSAGE,
  });
  const workbench = buildDossierWorkbench(dossier);

  assert.equal(workbench.synthesisGuide.status, "source_backed");
  assert.match(workbench.synthesisGuide.boundary, /不是作者原文证据/);
  assert.equal(workbench.synthesisGuide.points.length, 1);
  assert.equal(workbench.synthesisGuide.points[0].supportCount, 1);
  assert.equal(workbench.evidencePolicy.acceptedEvidence, 1);
});

test("dossier workbench builds a review queue from diagnosis, actions, inputs, and evidence", () => {
  const dossier = buildLearningDossier({
    id: "review-dossier",
    createdAt: "2026-05-25T10:20:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const workbench = buildDossierWorkbench(dossier);
  const queue = workbench.reviewQueue;

  assert.ok(queue);
  assert.ok(queue.progress.total >= 6);
  assert.equal(queue.progress.completed, 0);
  assert.ok(queue.items.some((item) => item.kind === "diagnosis" && item.label.includes("主图入口检查")));
  assert.ok(queue.items.some((item) => item.kind === "action" && item.label.includes("检查主图差异化")));
  assert.ok(queue.items.some((item) => item.kind === "input" && item.label.includes("补充")));
  assert.ok(queue.items.some((item) => item.kind === "evidence" && item.label.includes("你是如何解决转化率的？")));
  assert.ok(queue.nextItem);
  assert.ok(queue.nextItem.prompt);
});

test("updateDossierReviewState persists review progress without changing evidence", () => {
  const dossier = buildLearningDossier({
    id: "review-dossier",
    createdAt: "2026-05-25T10:20:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const firstReviewId = buildDossierWorkbench(dossier).reviewQueue.items[0].id;
  const updated = updateDossierReviewState(dossier, {
    checked: {
      [firstReviewId]: true,
      "unknown:item": true,
    },
    updatedAt: "2026-05-25T11:00:00.000Z",
  });
  const queue = buildDossierWorkbench(updated).reviewQueue;

  assert.equal(updated.reviewState.checked[firstReviewId], true);
  assert.equal(updated.reviewState.checked["unknown:item"], undefined);
  assert.equal(updated.reviewState.updatedAt, "2026-05-25T11:00:00.000Z");
  assert.equal(queue.progress.completed, 1);
  assert.equal(queue.items.find((item) => item.id === firstReviewId).done, true);
  assert.equal(updated.acceptedEvidence.length, dossier.acceptedEvidence.length);
  assert.equal(updated.rejectedEvidence.length, dossier.rejectedEvidence.length);
  assert.equal(updated.sources.length, dossier.sources.length);

  const cleared = updateDossierReviewState(updated, { checked: { [firstReviewId]: false } });
  assert.equal(cleared.reviewState.checked[firstReviewId], undefined);
  assert.equal(buildDossierWorkbench(cleared).reviewQueue.progress.completed, 0);
});

test("review queue keeps diagnosis item ids stable when new items are inserted", () => {
  const dossier = buildLearningDossier({
    id: "review-dossier",
    createdAt: "2026-05-25T10:20:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const originalItem = buildDossierWorkbench(dossier).reviewQueue.items.find((item) =>
    item.label.includes("对比前三名竞品主图"),
  );
  const changed = normalizeStoredDossier({
    ...dossier,
    diagnosisPanel: {
      ...dossier.diagnosisPanel,
      tracks: [
        {
          id: "inserted",
          label: "新增检查组",
          checks: [{ id: "new-check", label: "新增检查项" }],
        },
        ...dossier.diagnosisPanel.tracks,
      ],
    },
  });
  const changedItem = buildDossierWorkbench(changed).reviewQueue.items.find((item) =>
    item.label.includes("对比前三名竞品主图"),
  );

  assert.equal(changedItem.id, originalItem.id);
});

test("buildDossierOverview summarizes next actions across saved dossiers", () => {
  const first = buildLearningDossier({
    id: "overview-a",
    createdAt: "2026-05-25T09:00:00.000Z",
    question: "主图视觉点击率转化率怎么优化？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const firstItemId = buildDossierWorkbench(first).reviewQueue.items[0].id;
  const firstWithVerification = updateDossierBusinessVerificationState(first, {
    text: "主图是白底图，CTR 0.25%，CVR 5%，SP 广告 ACOS 45%，核心关键词 garlic press，竞品 ASIN B001234567",
    createdAt: "2026-05-25T10:30:00.000Z",
  });
  const firstUpdated = updateDossierReviewState(firstWithVerification, {
    checked: { [firstItemId]: true },
    updatedAt: "2026-05-25T11:00:00.000Z",
  });
  const firstWithExperiment = updateDossierExperimentResultState(firstUpdated, {
    text: "主图 A/B 小实验 7 天，CTR 从 0.25% 到 0.42%，CVR 从 5% 到 5.2%，ACOS 从 45% 到 38%",
    createdAt: "2026-05-25T13:00:00.000Z",
  });
  const second = buildLearningDossier({
    id: "overview-b",
    createdAt: "2026-05-25T10:00:00.000Z",
    question: "没有资料的问题",
    message: {
      role: "assistant",
      content: "这次没有从本地知识库里找到足够相关的资料。 【缺少来源】",
      evidenceChain: { claims: [{ id: "needs-source:0", type: "needs_source", text: "缺少来源" }] },
      learningCard: {
        intent: { label: "缺少来源问题" },
        takeaway: "先补具体产品数据。",
        missingInputs: ["产品链接", "关键词"],
        followUps: ["我补充产品链接后怎么判断？"],
      },
    },
  });
  const overview = buildDossierOverview([firstWithExperiment, second]);

  assert.equal(overview.totals.dossiers, 2);
  assert.ok(overview.totals.reviewTotal > 0);
  assert.equal(overview.totals.reviewCompleted, 2);
  assert.ok(overview.totals.selfTestTotal > 0);
  assert.equal(overview.totals.selfTestMastered, 0);
  assert.equal(overview.totals.businessVerificationRecords, 1);
  assert.equal(overview.totals.businessVerificationReady, 1);
  assert.equal(overview.totals.businessVerificationOpen, 1);
  assert.equal(overview.totals.businessVerificationDimensionsReady, 4);
  assert.equal(overview.totals.businessVerificationDimensionsTotal, 5);
  assert.equal(overview.totals.experimentResults, 1);
  assert.equal(overview.totals.experimentResultsPositive, 1);
  assert.match(overview.summary, /已保存 1 条业务材料/);
  assert.match(overview.summary, /仍有 1 个档案未补产品材料/);
  assert.ok(overview.nextItems.length > 0);
  assert.equal(overview.nextItems[0].dossierId, "overview-a");
  assert.ok(overview.nextItems[0].prompt);
  assert.ok(overview.researchMissions.some((item) => item.id === "overview-a" && item.stage === "review"));
  assert.ok(overview.researchMissions.some((item) => item.id === "overview-b" && item.stage === "evidence"));
  assert.ok(overview.researchMissions.every((item) => item.nextAction && item.reason && item.boundary));
  assert.ok(overview.studyMaterials.some((item) => item.type === "brief" && item.title.includes("摘要")));
  assert.ok(overview.studyMaterials.some((item) => item.type === "evidence_notes" && item.items.length > 0));
  assert.ok(overview.studyMaterials.some((item) => item.type === "faq" && item.items.length > 0));
  assert.ok(overview.studyMaterials.some((item) => item.type === "review_cards"));
  const visualTopic = overview.topicGroups.find((item) => item.id === "visual");
  assert.ok(visualTopic && visualTopic.evidenceCount > 0);
  assert.equal(visualTopic.evidenceStatus.level, "in_progress");
  assert.deepEqual(visualTopic.learningPath.map((item) => item.status), ["done", "done", "done", "current"]);
  assert.ok(visualTopic.supportedConclusions.some((item) => item.includes("已采纳原文支撑")));
  assert.ok(visualTopic.claimSupport.some((item) => item.claim.includes("主图") && item.evidence.length > 0));
  assert.equal(visualTopic.claimSupport[0].supportLevel, "source_supported");
  assert.ok(visualTopic.claimSupport[0].boundary.includes("业务材料"));
  assert.ok(visualTopic.sourcePackage);
  assert.equal(visualTopic.sourcePackage.status, "source_backed");
  assert.ok(visualTopic.sourcePackage.accepted.length > 0);
  assert.ok(visualTopic.sourcePackage.summary.includes("已采纳证据"));
  assert.ok(visualTopic.sourcePackage.nextPrompt.includes("已采纳来源"));
  assert.ok(visualTopic.sourcePackage.boundary.includes("候选来源未被采纳前不能当成结论依据"));
  assert.ok(visualTopic.validationHypotheses.some((item) => item.includes("需要用你的产品数据验证")));
  assert.ok(visualTopic.materialGaps.some((item) => item.includes("具体产品链接")));
  assert.ok(visualTopic.readingQueue.some((item) => item.id === "overview-a" && item.hasEvidence === true));
  assert.ok(Array.isArray(overview.learningPaths));
  const visualPath = overview.learningPaths.find((item) => item.topicId === "visual");
  assert.ok(visualPath);
  assert.equal(visualPath.topicLabel, "主图与视觉转化");
  assert.equal(visualPath.status, "in_progress");
  assert.equal(visualPath.currentStep.id, "review_understanding");
  assert.equal(visualPath.progress.done, 3);
  assert.equal(visualPath.progress.total, 4);
  assert.equal(visualPath.progress.percent, 75);
  assert.ok(visualPath.nextPrompt);
  assert.ok(visualPath.boundary.includes("不写入原始知识库"));
  assert.ok(visualPath.relatedDossiers.some((item) => item.id === "overview-a"));
  assert.equal(visualPath.candidateSourceCount, 1);
  assert.ok(overview.learningProducts.some((item) => item.type === "study_guide" && item.boundary.includes("原始知识库")));
  const studyGuide = overview.learningProducts.find((item) => item.type === "study_guide");
  assert.ok(studyGuide.sourceBackedClaims.some((item) => item.claim.includes("主图") && item.evidence[0].quote.includes("产品首图")));
  const firstClaimEvidence = studyGuide.sourceBackedClaims[0].evidence[0];
  assert.equal(firstClaimEvidence.dossierId, "overview-a");
  assert.equal(firstClaimEvidence.sourcePath, "跨境电商长期主义html/example.html");
  assert.equal(firstClaimEvidence.sourceKey, "跨境电商长期主义html/example.html");
  assert.ok(studyGuide.authorPerspectives.some((item) => item.author === "跨境电商长期主义" && item.sourceCount > 0));
  assert.ok(studyGuide.executionChecklist.some((item) => item.kind === "evidence" && item.done === true));
  assert.ok(studyGuide.executionChecklist.some((item) => item.kind === "experiment" && item.done === true));
  assert.ok(studyGuide.reviewQuestions.some((item) => item.question.includes("为什么") && item.expectedAnswer.includes("主图")));
  assert.ok(studyGuide.nextPrompt.includes("已采纳来源"));
  assert.ok(studyGuide.sourceControls.allowedSourceKeys.includes("跨境电商长期主义html/example.html"));
  assert.ok(studyGuide.sourceControls.selectedSources.some((item) => item.sourcePath === "跨境电商长期主义html/example.html"));
  assert.ok(studyGuide.boundary.includes("只有已采纳原文证据"));
  assert.equal(studyGuide.exportKind, "markdown_handout");
  assert.ok(studyGuide.downloadFilename.endsWith(".md"));
  assert.match(studyGuide.handoutMarkdown, /^# 主图与视觉转化学习讲义/m);
  assert.match(studyGuide.handoutMarkdown, /## 作者原文证据/);
  assert.match(studyGuide.handoutMarkdown, /产品首图极大程度上决定了点击率。/);
  assert.match(studyGuide.handoutMarkdown, /跨境电商长期主义/);
  assert.match(studyGuide.handoutMarkdown, /跨境电商长期主义html\/example.html/);
  assert.match(studyGuide.handoutMarkdown, /## 系统整理/);
  assert.match(studyGuide.handoutMarkdown, /## 用户业务材料与实验/);
  assert.match(studyGuide.handoutMarkdown, /这些不是作者原文证据/);
  assert.match(studyGuide.handoutMarkdown, /CTR 0\.25%/);
  assert.match(studyGuide.handoutMarkdown, /## 候选来源与限制/);
  const evidenceMarkdown = studyGuide.handoutMarkdown.split("## 作者原文证据")[1].split("## 系统整理")[0];
  assert.doesNotMatch(evidenceMarkdown, /CTR 0\.25%|ASIN|用户业务材料|实验复盘/);
  const evidenceReport = overview.learningProducts.find((item) => item.type === "evidence_report");
  assert.ok(evidenceReport);
  assert.ok(evidenceReport.title.includes("可审计"));
  assert.equal(evidenceReport.status, "source_backed");
  assert.ok(evidenceReport.claimAudits.some((item) => item.claim.includes("主图") && item.verdict === "source_supported"));
  assert.ok(evidenceReport.claimAudits[0].evidence.some((item) => item.quote.includes("产品首图")));
  assert.ok(evidenceReport.claimAudits[0].gaps.some((item) => item.includes("业务材料") || item.includes("实验")));
  assert.ok(evidenceReport.sourceLedger.some((item) => item.sourcePath === "跨境电商长期主义html/example.html" && item.claimCount > 0));
  assert.ok(evidenceReport.sourceControls.allowedSourceKeys.includes("跨境电商长期主义html/example.html"));
  assert.ok(evidenceReport.boundary.includes("不能替代原文"));
  assert.ok(overview.learningProducts.some((item) => item.type === "faq_pack" && item.questions.length > 0));
  assert.ok(overview.learningProducts.some((item) => item.type === "flashcards" && item.cards.length > 0));
  assert.ok(overview.learningProducts.some((item) => item.type === "research_brief"));
  assert.ok(overview.learningProducts.some((item) => item.type === "experiment_digest" && item.records.length > 0));
  assert.equal(overview.mastery.title, "亚马逊学习掌握面板");
  assert.equal(overview.mastery.status, "needs_evidence");
  assert.match(overview.mastery.summary, /2 个研究主题/);
  assert.ok(overview.mastery.score > 0);
  assert.ok(overview.mastery.stages.some((item) => item.id === "source_evidence" && item.done === 1 && item.total === 2));
  assert.ok(overview.mastery.stages.some((item) => item.id === "business_materials" && item.done === 1));
  assert.ok(overview.mastery.stages.some((item) => item.id === "experiments" && item.done === 1));
  assert.ok(overview.mastery.topics.some((item) => item.id === "visual" && item.percent === 75));
  assert.ok(overview.mastery.boundary.includes("不写入原始知识库"));
  assert.ok(overview.focusDossiers.some((item) => item.id === "overview-a" && item.progress.completed === 2));
  assert.ok(overview.evidenceGaps.some((item) => item.id === "overview-b" && item.reason.includes("缺少原文证据")));
  assert.ok(overview.summary.includes("2 个学习档案"));
});

test("buildDossierOverview does not create a formal study guide without accepted evidence", () => {
  const dossier = buildLearningDossier({
    id: "no-evidence-topic",
    createdAt: "2026-05-25T09:00:00.000Z",
    question: "主图怎么优化？",
    message: {
      role: "assistant",
      content: "这次没有从本地知识库里找到足够相关的资料。 【缺少来源】",
      evidenceChain: { claims: [{ id: "needs-source:0", type: "needs_source", text: "缺少来源" }] },
      learningCard: {
        intent: { label: "主图学习档案" },
        takeaway: "先补主图证据。",
        missingInputs: ["主图截图"],
        followUps: ["补主图截图后怎么判断？"],
      },
    },
  });
  const overview = buildDossierOverview([dossier]);
  const visualTopic = overview.topicGroups.find((item) => item.id === "visual");

  assert.ok(visualTopic);
  assert.equal(visualTopic.evidenceStatus.level, "needs_evidence");
  assert.equal(visualTopic.sourcePackage.status, "needs_source");
  assert.equal(visualTopic.sourcePackage.accepted.length, 0);
  assert.match(visualTopic.sourcePackage.summary, /缺可定位来源|还没有已采纳证据/);
  assert.equal(visualTopic.supportedConclusions.length, 0);
  assert.equal(visualTopic.claimSupport.length, 0);
  assert.deepEqual(visualTopic.learningPath.map((item) => item.status), ["current", "blocked", "blocked", "blocked"]);
  const visualPath = overview.learningPaths.find((item) => item.topicId === "visual");
  assert.ok(visualPath);
  assert.equal(visualPath.status, "needs_evidence");
  assert.equal(visualPath.currentStep.id, "read_sources");
  assert.equal(visualPath.progress.percent, 0);
  assert.equal(visualPath.candidateSourceCount, 0);
  assert.match(visualPath.nextAction, /标记真正有用的来源|补证据|来源/);
  assert.ok(visualTopic.materialGaps.some((item) => item.includes("先采纳")));
  assert.ok(overview.learningProducts.some((item) => item.type === "evidence_needed"));
  assert.equal(overview.learningProducts.some((item) => item.type === "study_guide"), false);
  assert.equal(overview.learningProducts.some((item) => item.type === "evidence_report"), false);
});

test("buildDossierOverview exposes a safe adoption route for candidate sources", () => {
  const messageWithCandidateOnly = {
    ...SAMPLE_MESSAGE,
    evidenceFeedback: {},
  };
  const dossier = buildLearningDossier({
    id: "candidate-source-topic",
    createdAt: "2026-05-25T09:30:00.000Z",
    question: "主图怎么优化？",
    message: messageWithCandidateOnly,
  });
  const overview = buildDossierOverview([dossier]);
  const visualTopic = overview.topicGroups.find((item) => item.id === "visual");
  const evidenceNeeded = overview.learningProducts.find((item) => item.type === "evidence_needed");

  assert.ok(visualTopic);
  assert.equal(visualTopic.evidenceCount, 0);
  assert.equal(visualTopic.sourcePackage.status, "needs_adoption");
  assert.equal(visualTopic.sourcePackage.candidates.length, 1);
  assert.equal(visualTopic.sourcePackage.selectedDossierId, "candidate-source-topic");
  assert.match(visualTopic.sourcePackage.nextPrompt, /候选来源/);
  assert.ok(evidenceNeeded);
  assert.equal(evidenceNeeded.selectedDossierId, "candidate-source-topic");
  assert.equal(evidenceNeeded.candidateSourceCount, 1);
  assert.equal(evidenceNeeded.sourcePackageStatus, "needs_adoption");
  assert.match(evidenceNeeded.sourceSummary, /候选来源/);
  assert.match(evidenceNeeded.boundary, /确认有用/);
  assert.equal(overview.learningProducts.some((item) => item.type === "study_guide"), false);
});

test("buildDossierOverview does not treat accepted snapshots without original source identity as source-backed", () => {
  const dossier = normalizeStoredDossier({
    id: "legacy-thin-source",
    createdAt: "2026-05-25T09:00:00.000Z",
    title: "旧档案",
    question: "主图怎么优化？",
    takeaway: "主图会影响点击率。",
    conclusions: ["主图会影响点击率"],
    acceptedEvidence: [
      {
        author: "跨境电商长期主义",
        quote: "产品首图极大程度上决定了点击率。",
      },
    ],
  });
  const overview = buildDossierOverview([dossier]);
  const visualTopic = overview.topicGroups.find((item) => item.id === "visual");

  assert.ok(visualTopic);
  assert.equal(visualTopic.evidenceCount, 0);
  assert.equal(visualTopic.sourcePackage.status, "needs_source");
  assert.equal(visualTopic.claimSupport.length, 0);
  assert.equal(overview.learningProducts.some((item) => item.type === "study_guide"), false);
});

test("buildDossierOverview aggregates mixed evidence states in one research topic", () => {
  const withEvidence = buildLearningDossier({
    id: "topic-mixed-a",
    createdAt: "2026-05-25T09:00:00.000Z",
    question: "主图怎么优化？",
    message: SAMPLE_MESSAGE,
  });
  const withoutEvidence = buildLearningDossier({
    id: "topic-mixed-b",
    createdAt: "2026-05-25T10:00:00.000Z",
    question: "主图点击率低怎么办？",
    message: {
      role: "assistant",
      content: "这次没有从本地知识库里找到足够相关的资料。 【缺少来源】",
      evidenceChain: { claims: [{ id: "needs-source:0", type: "needs_source", text: "缺少来源" }] },
      learningCard: {
        intent: { label: "主图点击率学习档案" },
        takeaway: "先补主图来源证据。",
        nextActions: ["对比前三名竞品主图"],
        missingInputs: ["竞品主图截图"],
      },
    },
  });
  const overview = buildDossierOverview([withEvidence, withoutEvidence]);
  const visualTopic = overview.topicGroups.find((item) => item.id === "visual");

  assert.ok(visualTopic);
  assert.equal(visualTopic.dossierCount, 2);
  assert.equal(visualTopic.evidenceCount, 1);
  assert.equal(visualTopic.evidenceStatus.level, "source_backed");
  assert.equal(visualTopic.sourcePackage.status, "source_backed");
  assert.ok(visualTopic.sourcePackage.accepted.length > 0);
  assert.deepEqual(visualTopic.learningPath.map((item) => item.status), ["done", "current", "blocked", "blocked"]);
  assert.equal(visualTopic.supportedConclusions.length, 1);
  assert.equal(visualTopic.claimSupport.length, 1);
  assert.ok(visualTopic.claimSupport.every((item) => item.evidenceCount === 1));
  assert.ok(visualTopic.validationHypotheses.some((item) => item.includes("对比前三名竞品主图")));
  assert.ok(visualTopic.materialGaps.some((item) => item.includes("竞品主图截图")));
  assert.ok(visualTopic.readingQueue.some((item) => item.id === "topic-mixed-a" && item.hasEvidence === true));
  assert.ok(visualTopic.readingQueue.some((item) => item.id === "topic-mixed-b" && item.hasEvidence === false));
  const visualPath = overview.learningPaths.find((item) => item.topicId === "visual");
  assert.ok(visualPath);
  assert.equal(visualPath.status, "source_backed");
  assert.equal(visualPath.currentStep.id, "add_materials");
  assert.equal(visualPath.progress.done, 1);
  assert.equal(visualPath.progress.percent, 25);
  assert.match(visualPath.nextAction, /产品材料|业务材料/);
  assert.match(visualPath.materialTemplate.text, /产品\/ASIN/);
  assert.match(visualPath.materialTemplate.text, /主图现状/);
  assert.match(visualPath.materialTemplate.text, /CTR/);
  assert.match(visualPath.materialTemplate.text, /CVR/);
  assert.match(visualPath.materialTemplate.text, /核心关键词/);
  assert.match(visualPath.materialTemplate.text, /竞品/);
  assert.match(visualPath.materialTemplate.boundary, /不是作者原文证据/);
});

test("buildDossierOverview only links claim support to matching evidence", () => {
  const message = {
    ...SAMPLE_MESSAGE,
    learningCard: {
      ...SAMPLE_MESSAGE.learningCard,
      takeaway: "主图会影响点击率。",
      conclusions: ["主图会影响点击率", "广告预算要先降低"],
      nextActions: ["降低广告预算"],
    },
  };
  const dossier = buildLearningDossier({
    id: "claim-linking",
    createdAt: "2026-05-25T09:00:00.000Z",
    question: "主图和广告怎么判断？",
    message,
  });
  const overview = buildDossierOverview([dossier]);
  const visualTopic = overview.topicGroups.find((item) => item.id === "visual");

  assert.ok(visualTopic);
  assert.ok(visualTopic.claimSupport.some((item) => item.claim.includes("主图")));
  assert.equal(visualTopic.claimSupport.some((item) => item.claim.includes("广告预算")), false);
  assert.ok(visualTopic.validationHypotheses.some((item) => item.includes("降低广告预算")));
});

test("buildDossierOverview does not treat overextended conclusions as directly source-supported", () => {
  const message = {
    ...SAMPLE_MESSAGE,
    learningCard: {
      ...SAMPLE_MESSAGE.learningCard,
      takeaway: "只要换主图就一定提高转化率，不需要看价格评价。",
      conclusions: ["只要换主图就一定提高转化率，不需要看价格评价。"],
      nextActions: ["验证主图点击入口"],
    },
    evidenceFeedback: {
      "source-evidence:0": "useful",
    },
  };
  const dossier = buildLearningDossier({
    id: "overclaim",
    createdAt: "2026-05-25T09:00:00.000Z",
    question: "主图怎么优化？",
    message,
  });
  const overview = buildDossierOverview([dossier]);
  const visualTopic = overview.topicGroups.find((item) => item.id === "visual");

  assert.ok(visualTopic);
  assert.equal(visualTopic.claimSupport.some((item) => item.claim.includes("一定提高转化率")), false);
  assert.equal(overview.learningProducts.some((item) =>
    item.type === "study_guide" &&
    item.sourceBackedClaims.some((claim) => claim.claim.includes("一定提高转化率"))
  ), false);
  assert.equal(overview.learningProducts.some((item) =>
    item.type === "evidence_report" &&
    item.claimAudits.some((claim) => claim.claim.includes("一定提高转化率"))
  ), false);
});

test("buildDossierOverview does not treat path-like sourceKey without source identity as traceable", () => {
  const dossier = normalizeStoredDossier({
    id: "fake-source-key-only",
    createdAt: "2026-05-25T09:00:00.000Z",
    title: "伪路径证据",
    question: "主图怎么优化？",
    takeaway: "主图是点击率入口。",
    conclusions: ["主图是点击率入口。"],
    acceptedEvidence: [
      {
        id: "legacy-fake-path",
        author: "跨境电商长期主义",
        title: "你是如何解决转化率的？",
        quote: "产品首图极大程度上决定了点击率。",
        sourceKey: "fake/path.html",
      },
    ],
    sources: [],
    nextActions: ["检查主图差异化"],
    followUps: [],
    missingInputs: [],
  });
  const overview = buildDossierOverview([dossier]);
  const visualTopic = overview.topicGroups.find((item) => item.id === "visual");

  assert.ok(visualTopic);
  assert.equal(visualTopic.evidenceCount, 0);
  assert.equal(visualTopic.sourcePackage.status, "needs_source");
  assert.equal(visualTopic.claimSupport.length, 0);
  assert.equal(overview.learningProducts.some((item) => item.type === "study_guide"), false);
  assert.equal(overview.learningProducts.some((item) => item.type === "evidence_report"), false);
});

test("buildDossierOverview does not promote user metric judgments as author-supported claims", () => {
  const message = {
    ...SAMPLE_MESSAGE,
    learningCard: {
      ...SAMPLE_MESSAGE.learningCard,
      takeaway: "你的 CTR 0.25% 说明当前主图已经明显不合格。",
      conclusions: ["你的 CTR 0.25% 说明当前主图已经明显不合格。"],
      nextActions: ["先验证主图点击入口"],
    },
  };
  const dossier = buildLearningDossier({
    id: "business-judgment",
    createdAt: "2026-05-25T09:00:00.000Z",
    question: "我的主图 CTR 0.25% 怎么办？",
    message,
  });
  const overview = buildDossierOverview([dossier]);
  const visualTopic = overview.topicGroups.find((item) => item.id === "visual");

  assert.ok(visualTopic);
  assert.equal(visualTopic.claimSupport.some((item) => item.claim.includes("0.25%")), false);
  assert.ok(visualTopic.validationHypotheses.some((item) => item.includes("先验证主图点击入口")));
});

test("buildDossierOverview turns saved dossiers into research missions", () => {
  const scoped = buildLearningDossier({
    id: "mission-scope",
    createdAt: "2026-05-25T09:00:00.000Z",
    question: "主图怎么优化？",
    message: SAMPLE_MESSAGE,
    sourceControls: { allowedAuthors: ["跨境电商长期主义"] },
  });
  const withMaterials = updateDossierBusinessVerificationState(scoped, {
    text: "主图是白底图，CTR 0.25%，CVR 5%，SP 广告 ACOS 45%，核心关键词 garlic press，竞品 ASIN B001234567",
    createdAt: "2026-05-25T10:00:00.000Z",
  });
  const overview = buildDossierOverview([withMaterials]);
  const mission = overview.researchMissions[0];

  assert.equal(mission.id, "mission-scope");
  assert.equal(mission.sourceScope, "只看 跨境电商长期主义");
  assert.equal(mission.stage, "verification");
  assert.match(mission.nextAction, /补齐|验证|主图|点击率|转化率|广告|关键词/);
  assert.ok(mission.prompt);
  assert.equal(mission.materialRecords, 1);
  assert.equal(mission.experimentResults, 0);
  const visualTopic = overview.topicGroups.find((item) => item.id === "visual");
  assert.equal(visualTopic.evidenceStatus.level, "needs_experiment");
  assert.ok(visualTopic.materialGaps.some((item) => item.includes("小实验")));
  const visualPath = overview.learningPaths.find((item) => item.topicId === "visual");
  assert.ok(visualPath);
  assert.equal(visualPath.status, "needs_experiment");
  assert.equal(visualPath.currentStep.id, "run_experiment");
  assert.match(visualPath.nextAction, /验证|小实验|复盘/);
  assert.match(visualPath.experimentTemplate.text, /实验名称/);
  assert.match(visualPath.experimentTemplate.text, /CTR 前\/后/);
  assert.match(visualPath.experimentTemplate.text, /CVR 前\/后/);
  assert.match(visualPath.experimentTemplate.text, /ACOS 前\/后/);
  assert.match(visualPath.experimentTemplate.boundary, /不是作者原文证据/);
});

test("review queue requires real material records for input gaps", () => {
  const dossier = buildLearningDossier({
    id: "review-material-gate",
    createdAt: "2026-05-25T09:00:00.000Z",
    question: "主图怎么优化？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const workbench = buildDossierWorkbench(dossier);
  const inputItem = workbench.reviewQueue.items.find((item) => item.kind === "input" && item.label.includes("产品链接"));

  assert.ok(inputItem);
  assert.equal(inputItem.canManualComplete, false);
  assert.equal(inputItem.done, false);

  const checked = updateDossierReviewState(dossier, { checked: { [inputItem.id]: true } });
  assert.equal(checked.reviewState.checked[inputItem.id], undefined);

  const withMaterial = updateDossierBusinessVerificationState(dossier, {
    text: "产品链接：https://example.com/product，主图是白底图，CTR 0.25%，CVR 5%",
    createdAt: "2026-05-25T10:00:00.000Z",
  });
  const nextWorkbench = buildDossierWorkbench(withMaterial);
  const completedInputItem = nextWorkbench.reviewQueue.items.find((item) => item.id === inputItem.id);
  assert.equal(completedInputItem.done, true);
  assert.equal(completedInputItem.completionLabel, "已补材料");
});

test("buildDossierWorkbench exposes validation pack progress before and after business verification", () => {
  const dossier = buildLearningDossier({
    id: "validation-pack-progress",
    createdAt: "2026-05-25T09:00:00.000Z",
    question: "主图怎么优化？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });

  const initial = buildDossierWorkbench(dossier).validationPack;
  assert.equal(initial.status, "pending_materials");
  assert.match(initial.summary, /还没有补真实产品材料/);
  assert.equal(initial.sourceStatus, "source_backed");
  assert.equal(initial.counts.hypotheses, 1);
  assert.equal(initial.counts.dataRequests, 2);
  assert.ok(initial.nextPrompt.includes("验证数据"));

  const withMaterial = updateDossierBusinessVerificationState(dossier, {
    text: "主图是白底图，CTR 0.25%，CVR 5%，SP 广告 ACOS 45%，核心关键词 garlic press，竞品 ASIN B001234567",
    createdAt: "2026-05-25T10:00:00.000Z",
  });
  const afterMaterial = buildDossierWorkbench(withMaterial).validationPack;
  assert.equal(afterMaterial.status, "materials_ready");
  assert.match(afterMaterial.summary, /已保存 1 条业务材料/);

  const withExperiment = updateDossierExperimentResultState(withMaterial, {
    text: "主图 A/B 小实验 7 天，CTR 从 0.25% 到 0.42%，CVR 从 5% 到 5.2%",
    createdAt: "2026-05-25T11:00:00.000Z",
  });
  const reviewed = buildDossierWorkbench(withExperiment).validationPack;
  assert.equal(reviewed.status, "experiment_reviewed");
  assert.match(reviewed.summary, /已回填 1 条实验复盘/);
});

test("buildDossierWorkbench keeps no-source validation packs conservative", () => {
  const dossier = buildLearningDossier({
    id: "validation-pack-no-source",
    createdAt: "2026-05-25T09:00:00.000Z",
    question: "怎么优化？",
    message: {
      ...SAMPLE_MESSAGE,
      sources: [],
      evidenceChain: { claims: [] },
      validationPack: {
        ...SAMPLE_VALIDATION_PACK,
        status: "needs_source",
        summary: "这轮缺少作者原文证据，只能先列验证方向，不能直接沉淀成结论。",
        boundary: "这轮没有作者原文证据；用户数据和学习档案只能提出验证方向，不能替代作者原文证据。",
        hypotheses: [],
        dataRequests: [{ id: "source-scope", label: "更具体的作者资料问题", why: "先命中作者原文。" }],
        experiments: [{ id: "source-first", title: "先补作者证据再做判断", steps: ["换一个更具体的问题"] }],
        decisionRules: [{ if: "没有作者原文来源", then: "只当成待验证方向。" }],
      },
    },
  });

  const pack = buildDossierWorkbench(dossier).validationPack;
  assert.equal(pack.sourceStatus, "needs_source");
  assert.equal(pack.status, "needs_source");
  assert.match(pack.summary, /先补作者来源/);
  assert.equal(pack.counts.hypotheses, 0);
  assert.equal(dossier.acceptedEvidence.length, 0);
});

test("buildDossierOverview counts dossiers without business verification, not missing records", () => {
  const first = buildLearningDossier({
    id: "overview-open-a",
    createdAt: "2026-05-25T09:00:00.000Z",
    question: "主图怎么优化？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const firstOne = updateDossierBusinessVerificationState(first, {
    text: "主图是白底图，CTR 0.25%，CVR 5%，SP 广告 ACOS 45%，核心关键词 garlic press，竞品 ASIN B001234567",
    createdAt: "2026-05-25T10:00:00.000Z",
  });
  const firstTwo = updateDossierBusinessVerificationState(firstOne, {
    text: "广告 CTR 0.3%，CVR 4%，ACOS 42%，关键词 garlic press stainless，竞品 ASIN B009999999",
    createdAt: "2026-05-25T11:00:00.000Z",
  });
  const second = buildLearningDossier({
    id: "overview-open-b",
    createdAt: "2026-05-25T09:30:00.000Z",
    question: "广告怎么优化？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const overview = buildDossierOverview([firstTwo, second]);

  assert.equal(overview.totals.dossiers, 2);
  assert.equal(overview.totals.businessVerificationRecords, 2);
  assert.equal(overview.totals.businessVerificationOpen, 1);
});

test("dossier workbench builds source-aware self test cards", () => {
  const dossier = buildLearningDossier({
    id: "self-test-dossier",
    createdAt: "2026-05-25T10:20:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const selfTest = buildDossierWorkbench(dossier).selfTest;

  assert.ok(selfTest);
  assert.ok(selfTest.items.length >= 4);
  assert.equal(selfTest.progress.total, selfTest.items.length);
  assert.equal(selfTest.progress.mastered, 0);
  assert.ok(selfTest.items.some((item) => item.kind === "takeaway" && item.answer.includes("先处理主图")));
  assert.ok(selfTest.items.some((item) => item.kind === "evidence" && item.answer.includes("产品首图")));
  assert.ok(selfTest.items.every((item) => item.question && item.answer && item.explanation));
});

test("updateDossierSelfTestState persists mastered cards without changing evidence", () => {
  const dossier = buildLearningDossier({
    id: "self-test-dossier",
    createdAt: "2026-05-25T10:20:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const firstCardId = buildDossierWorkbench(dossier).selfTest.items[0].id;
  const updated = updateDossierSelfTestState(dossier, {
    mastered: {
      [firstCardId]: true,
      "fake-card": true,
    },
    updatedAt: "2026-05-25T12:00:00.000Z",
  });
  const selfTest = buildDossierWorkbench(updated).selfTest;

  assert.equal(updated.selfTestState.mastered[firstCardId], true);
  assert.equal(updated.selfTestState.mastered["fake-card"], undefined);
  assert.equal(updated.selfTestState.updatedAt, "2026-05-25T12:00:00.000Z");
  assert.equal(selfTest.progress.mastered, 1);
  assert.equal(selfTest.items.find((item) => item.id === firstCardId).mastered, true);
  assert.equal(updated.acceptedEvidence.length, dossier.acceptedEvidence.length);
  assert.equal(updated.sources.length, dossier.sources.length);

  const cleared = updateDossierSelfTestState(updated, { mastered: { [firstCardId]: false } });
  assert.equal(cleared.selfTestState.mastered[firstCardId], undefined);
  assert.equal(buildDossierWorkbench(cleared).selfTest.progress.mastered, 0);
});

test("updateDossierBusinessVerificationState saves user materials without changing evidence", () => {
  const dossier = buildLearningDossier({
    id: "business-verification-dossier",
    createdAt: "2026-05-25T10:40:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });

  const updated = updateDossierBusinessVerificationState(dossier, {
    text: "主图是白底图，CTR 0.25%，CVR 5%，SP 广告 ACOS 45%，核心关键词 garlic press，竞品 ASIN B001234567",
    createdAt: "2026-05-25T12:30:00.000Z",
  });
  const record = updated.businessVerificationRecords[0];
  const workbench = buildDossierWorkbench(updated);

  assert.equal(updated.businessVerificationRecords.length, 1);
  assert.equal(record.status, "ready");
  assert.match(record.rawText, /CTR 0.25%/);
  assert.ok(record.sections.find((section) => section.id === "metrics").items.some((item) => item.includes("CTR")));
  assert.ok(record.sections.find((section) => section.id === "ads").items.some((item) => item.includes("ACOS")));
  assert.ok(record.sections.find((section) => section.id === "keywords").items.some((item) => item.includes("ASIN")));
  assert.match(record.diagnosticPrompt, /先改哪一块/);
  assert.match(record.diagnosticPrompt, /作者原文证据/);
  assert.match(record.diagnosticPrompt, /用户业务材料/);
  assert.match(record.diagnosticPrompt, /实验复盘/);
  assert.match(record.diagnosticPrompt, /不能把用户材料当成作者原文证据/);
  assert.match(record.caution, /不是本地资料证据/);
  assert.match(record.caution, /不写入原始知识库/);
  assert.equal(updated.acceptedEvidence.length, dossier.acceptedEvidence.length);
  assert.equal(updated.sources.length, dossier.sources.length);
  assert.equal(updated.excludedSources.length, dossier.excludedSources.length);
  assert.equal(workbench.businessVerification.records.length, 1);
  assert.equal(workbench.businessVerification.readyRecords, 1);
});

test("buildDossierWorkbench summarizes business verification dimensions", () => {
  const dossier = buildLearningDossier({
    id: "business-verification-dimensions",
    createdAt: "2026-05-25T10:45:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });

  const first = updateDossierBusinessVerificationState(dossier, {
    text: "主图是白底图，CTR 0.25%，CVR 5%，SP 广告 ACOS 45%，核心关键词 garlic press，竞品 ASIN B001234567",
    createdAt: "2026-05-25T12:30:00.000Z",
  });
  const updated = updateDossierBusinessVerificationState(first, {
    text: "Listing 标题包含 garlic press，价格 19.99，评价 12 条，评分 4.2",
    createdAt: "2026-05-25T12:35:00.000Z",
  });
  const verification = buildDossierWorkbench(updated).businessVerification;

  assert.equal(verification.coverage.total, 5);
  assert.equal(verification.coverage.ready, 5);
  assert.equal(verification.coverage.percent, 100);
  assert.ok(verification.coverage.complete < verification.coverage.total);
  assert.ok(verification.coverage.completePercent < 100);
  assert.equal(verification.nextDimension.id, "visual");
  assert.equal(verification.dimensions.length, 5);
  assert.equal(verification.dimensions.find((item) => item.id === "visual").status, "ready");
  assert.equal(verification.dimensions.find((item) => item.id === "listing").status, "ready");
  assert.ok(verification.dimensions.find((item) => item.id === "ads").latestItems.some((item) => item.includes("ACOS")));
  assert.ok(verification.dimensions.find((item) => item.id === "keywords").latestItems.some((item) => item.includes("ASIN")));
  assert.match(verification.summary, /材料线索覆盖/);
  assert.match(verification.caution, /不代表产品验证完成/);
});

test("buildDossierWorkbench turns business materials into a validation plan", () => {
  const dossier = buildLearningDossier({
    id: "business-validation-plan",
    createdAt: "2026-05-25T10:45:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const updated = updateDossierBusinessVerificationState(dossier, {
    text: "主图是白底图，CTR 0.25%，CVR 5%，SP 广告 ACOS 45%，核心关键词 garlic press，竞品 ASIN B001234567",
    createdAt: "2026-05-25T12:30:00.000Z",
  });
  const plan = buildDossierWorkbench(updated).businessVerification.validationPlan;

  assert.equal(plan.priorityDimension.id, "visual");
  assert.match(plan.summary, /当前第一个待补维度/);
  assert.ok(plan.materialChecklist.length >= 4);
  assert.ok(plan.materialChecklist.some((item) => item.dimensionId === "visual" && item.label.includes("主图")));
  assert.ok(plan.decisionGates.some((gate) => gate.dimensionId === "metrics" && gate.metric.includes("CTR")));
  assert.ok(plan.decisionGates.every((gate) => gate.source === "业务操作假设"));
  assert.ok(plan.decisionGates.every((gate) => gate.boundary.includes("不是作者原文")));
  assert.ok(plan.experiments.some((experiment) => experiment.dimensionId === "visual" && experiment.steps.length >= 3));
  assert.match(plan.experiments[0].prompt, /实验/);
  assert.match(plan.priorityDimension.prompt, /作者原文证据/);
  assert.match(plan.priorityDimension.prompt, /用户业务材料只用于验证/);
  assert.match(plan.experiments[0].prompt, /作者原文证据/);
  assert.match(plan.experiments[0].prompt, /实验复盘不能改写作者资料/);
  assert.match(plan.caution, /不是结论/);
});

test("buildDossierWorkbench does not create validation gates before business materials exist", () => {
  const dossier = buildLearningDossier({
    id: "empty-business-validation-plan",
    createdAt: "2026-05-25T10:45:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const plan = buildDossierWorkbench(dossier).businessVerification.validationPlan;

  assert.equal(plan.priorityDimension, null);
  assert.equal(plan.materialChecklist.length, 0);
  assert.equal(plan.decisionGates.length, 0);
  assert.equal(plan.experiments.length, 0);
  assert.match(plan.summary, /先保存真实产品材料/);
});

test("updateDossierExperimentResultState saves experiment results without changing evidence", () => {
  const dossier = buildLearningDossier({
    id: "experiment-result-dossier",
    createdAt: "2026-05-25T10:55:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const withMaterials = updateDossierBusinessVerificationState(dossier, {
    text: "主图是白底图，CTR 0.25%，CVR 5%，SP 广告 ACOS 45%，核心关键词 garlic press，竞品 ASIN B001234567",
    createdAt: "2026-05-25T12:30:00.000Z",
  });
  const updated = updateDossierExperimentResultState(withMaterials, {
    text: "主图 A/B 小实验 7 天，CTR 从 0.25% 到 0.42%，CVR 从 5% 到 5.2%，ACOS 从 45% 到 38%",
    createdAt: "2026-05-25T13:00:00.000Z",
  });
  const result = updated.experimentResultRecords[0];
  const workbench = buildDossierWorkbench(updated);

  assert.equal(updated.experimentResultRecords.length, 1);
  assert.equal(result.outcome, "positive");
  assert.ok(result.metrics.some((metric) => metric.name === "CTR" && metric.direction === "up"));
  assert.ok(result.metrics.some((metric) => metric.name === "ACOS" && metric.direction === "down"));
  assert.match(result.nextAction, /继续验证|扩大|复核/);
  assert.match(result.caution, /不是作者原文证据/);
  assert.equal(updated.acceptedEvidence.length, withMaterials.acceptedEvidence.length);
  assert.equal(updated.sources.length, withMaterials.sources.length);
  assert.equal(workbench.businessVerification.experimentResults.records.length, 1);
  assert.equal(workbench.businessVerification.experimentResults.summary.outcome, "positive");
});

test("updateDossierExperimentResultState marks thin experiment results as inconclusive", () => {
  const dossier = buildLearningDossier({
    id: "thin-experiment-result",
    createdAt: "2026-05-25T10:56:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const updated = updateDossierExperimentResultState(dossier, {
    text: "我换了一版主图，感觉还行",
    createdAt: "2026-05-25T13:10:00.000Z",
  });
  const result = updated.experimentResultRecords[0];

  assert.equal(result.outcome, "inconclusive");
  assert.equal(result.metrics.length, 0);
  assert.match(result.missing.join("、"), /时间窗口|前后数据/);
});

test("updateDossierExperimentResultState keeps partial metric wins conservative", () => {
  const dossier = buildLearningDossier({
    id: "partial-experiment-result",
    createdAt: "2026-05-25T10:57:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const updated = updateDossierExperimentResultState(dossier, {
    text: "主图 A/B 小实验 7 天，CTR 从 0.25% 到 0.42%",
    createdAt: "2026-05-25T13:20:00.000Z",
  });
  const result = updated.experimentResultRecords[0];

  assert.equal(result.outcome, "partial_positive");
  assert.match(result.summary, /局部正向/);
  assert.match(result.missing.join("、"), /CVR 前后变化|ACOS 前后变化/);
  assert.match(result.nextAction, /补齐|确认/);
  assert.doesNotMatch(result.nextAction, /扩大/);
});

test("updateDossierExperimentResultState ignores empty experiment results", () => {
  const dossier = buildLearningDossier({
    id: "empty-experiment-result",
    createdAt: "2026-05-25T10:57:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const updated = updateDossierExperimentResultState(dossier, {
    text: "   ",
    createdAt: "2026-05-25T13:20:00.000Z",
  });

  assert.equal(updated.experimentResultRecords.length, 0);
  assert.equal(updated.acceptedEvidence.length, dossier.acceptedEvidence.length);
});

test("buildDossierWorkbench points to the first missing business verification dimension", () => {
  const dossier = buildLearningDossier({
    id: "business-verification-missing-dimension",
    createdAt: "2026-05-25T10:46:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });
  const updated = updateDossierBusinessVerificationState(dossier, {
    text: "CTR 0.25%，CVR 5%，最近 7 天 session 1200",
    createdAt: "2026-05-25T12:45:00.000Z",
  });
  const verification = buildDossierWorkbench(updated).businessVerification;
  const visual = verification.dimensions.find((item) => item.id === "visual");

  assert.equal(verification.coverage.total, 5);
  assert.equal(verification.coverage.ready, 1);
  assert.equal(verification.nextDimension.id, "visual");
  assert.equal(visual.status, "missing");
  assert.ok(visual.missing.includes("主图/副图截图"));
  assert.match(visual.prompt, /主图\/视觉/);
  assert.match(visual.prompt, /还缺/);
});

test("updateDossierBusinessVerificationState ignores empty user materials", () => {
  const dossier = buildLearningDossier({
    id: "empty-business-verification",
    createdAt: "2026-05-25T10:50:00.000Z",
    question: "我应该先改哪一块？",
    message: PRODUCT_DIAGNOSIS_MESSAGE,
  });

  const updated = updateDossierBusinessVerificationState(dossier, {
    text: "   ",
    createdAt: "2026-05-25T12:40:00.000Z",
  });

  assert.equal(updated.businessVerificationRecords.length, 0);
  assert.equal(updated.acceptedEvidence.length, dossier.acceptedEvidence.length);
});

test("buildDossierWorkbench creates a local checklist and question pack", () => {
  const dossier = buildLearningDossier({
    id: "dossier-test",
    createdAt: "2026-05-25T10:10:00.000Z",
    question: "主图视觉点击率转化率怎么优化？",
    message: SAMPLE_MESSAGE,
    sourceControls: {
      excludedSourceKeys: ["跨境电商长期主义html/blocked-example.html"],
    },
  });
  const workbench = buildDossierWorkbench(dossier);

  assert.equal(workbench.title, "视觉转化学习档案");
  assert.ok(workbench.checklist.some((item) => item.kind === "action" && item.label === "检查主图差异化"));
  assert.ok(workbench.checklist.some((item) => item.kind === "input" && item.label === "补充：具体产品链接"));
  assert.ok(workbench.checklist.some((item) => item.kind === "evidence" && item.label.includes("你是如何解决转化率的？")));
  assert.ok(workbench.questionPack.some((item) => item.question === "我的主图应该怎么改？"));
  assert.ok(workbench.questionPack.some((item) => item.intent === "next_best_step"));
  assert.equal(workbench.evidencePolicy.acceptedEvidence, 1);
  assert.equal(workbench.evidencePolicy.rejectedEvidence, 1);
  assert.equal(workbench.evidencePolicy.excludedSources, 1);
  assert.equal(workbench.evidencePolicy.acceptedFromExcluded, false);
});

test("buildProductIntake classifies pasted product facts into diagnostic inputs", () => {
  const dossier = buildLearningDossier({
    id: "dossier-test",
    question: "主图视觉点击率转化率怎么优化？",
    message: SAMPLE_MESSAGE,
  });
  const intake = buildProductIntake({
    text: [
      "主图是白底图，和竞品差不多",
      "CTR 0.25%，CVR 5%，最近 7 天 session 1200",
      "Listing 标题写了核心关键词，五点比较短",
      "SP 广告 ACOS 45%，预算每天 20 美金",
      "核心关键词 garlic press，竞品 ASIN B001234",
    ].join("\n"),
  }, dossier);

  assert.match(intake.summary, /已识别/);
  assert.ok(intake.sections.find((section) => section.id === "visual").items.some((item) => item.includes("主图")));
  assert.ok(intake.sections.find((section) => section.id === "metrics").items.some((item) => item.includes("CTR")));
  assert.ok(intake.sections.find((section) => section.id === "listing").items.some((item) => item.includes("Listing")));
  assert.ok(intake.sections.find((section) => section.id === "ads").items.some((item) => item.includes("ACOS")));
  assert.ok(intake.sections.find((section) => section.id === "keywords").items.some((item) => item.includes("ASIN")));
  assert.match(intake.diagnosticPrompt, /先改哪一块/);
  assert.match(intake.caution, /不会写入原始知识库/);
});

test("buildProductIntake keeps vague input explicit instead of over-diagnosing", () => {
  const intake = buildProductIntake({ text: "最近感觉表现不太好" }, {});

  assert.match(intake.summary, /已识别|还没有识别/);
  assert.ok(intake.sections.find((section) => section.id === "other").items.includes("最近感觉表现不太好"));
  assert.ok(intake.missing.includes("CTR"));
  assert.match(intake.diagnosticPrompt, /仍缺信息/);
});
