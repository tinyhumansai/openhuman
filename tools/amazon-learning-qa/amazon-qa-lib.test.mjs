import test from "node:test";
import assert from "node:assert/strict";

import {
  buildQaPayload,
  buildAnswerGraph,
  buildEvidenceChain,
  buildLearningCard,
  buildLearningQueue,
  buildKnowledgeHealthSummary,
  buildKnowledgeReadinessSummary,
  buildProductInputSummary,
  buildRetrievalQuery,
  buildSourceContextFromArticle,
  buildAuthorPerspectiveRoom,
  buildTopicSourceTree,
  buildWorkflowIntent,
  normalizeLearningMemoryReminder,
  normalizeContextText,
  parseOpenHumanContext,
  scoreSentences,
} from "./amazon-qa-lib.mjs";

const SAMPLE_CONTEXT = `Query: 新品选品应该如何判断是否值得做？

飞翔的波波 2025-09-15 从0到1判断产品能不能做——亚马逊市场分析全攻略: # 从0到1判断产品能不能做——亚马逊市场分析全攻略
作者：飞翔的波波
发布时间：2025-09-15 23:03:05
原文链接：https://mp.weixin.qq.com/s/example1
来源文件：飞翔的波波html/example.html
市场容量决定了销量的天花板。如果市场太小，说明没啥需求。
我们可以从核心关键词的月搜索量、头部卖家的月销量、新品数量占比判断市场机会。
如果前10名占据整个品类60%~80%的销量，说明该类目已被大卖牢牢把控。

跨境电商长期主义 2025-01-16 很多人一辈子就跌在低水平重复上。: # 很多人一辈子就跌在低水平重复上。
作者：跨境电商长期主义
发布时间：2025-01-16 00:01:04
原文链接：https://mp.weixin.qq.com/s/example2
来源文件：跨境电商长期主义html/example.html
我判断一个事儿值不值得做，逻辑很简单：我是否会重复利用这项技能或者资源。
我们目前聚焦的点都是可以被重复利用并且能够产生相对不错价值的东西。`;

const MAIN_IMAGE_CONTEXT = `Query: 主图视觉点击率转化率怎么优化？

跨境电商长期主义 2022-06-18 你是如何解决转化率的？: # 你是如何解决转化率的？
作者：跨境电商长期主义
发布时间：2022-06-18 08:50:45
原文链接：https://mp.weixin.qq.com/s/example4
来源文件：跨境电商长期主义html/example.html
产品首图极大程度上决定了点击率，但是如果没有点击率，转化率就成了一个没有必要的数据指标。
千万不要跟别人采取一样的策略，最好是比别人多点什么，可以是配件，可以是好的包装，也可以是独特赠品。
进入商品页面之后的提升，一方面靠文案和视觉，还有很重要的一点就是对比。

跨境电商长期主义 2026-03-25 很多亚马逊上的产品，压根不具备接客资格。: # 很多亚马逊上的产品，压根不具备接客资格。
作者：跨境电商长期主义
发布时间：2026-03-25 18:01:29
原文链接：https://mp.weixin.qq.com/s/example5
来源文件：跨境电商长期主义html/example.html
主图是唯一一个纯视觉元素。一个真正具备接客资格的图片体系，必须包含差异化的主图、场景化的副图、尺寸细节对比图和核心功能演示图。
广告是放大器，不是救命稻草。转化取决于 Listing 质量、产品力和评价基础。`;

const PERSONA_CONTEXT = `Query: 人群画像应该怎么构建？有哪些实操指导建议

跨境电商长期主义 2026-02-21 假如你的创业起步预算比较少。: # 假如你的创业起步预算比较少。
作者：跨境电商长期主义
发布时间：2026-02-21 00:00:00
原文链接：https://mp.weixin.qq.com/s/persona1
来源文件：跨境电商长期主义html/persona1.html
假设我们要开发一条汽配产品的品线，最好的方式并不是直接筛选一款我们认为成功率最高的产品，而是先根据我们的选品条件找出售价，回款金额，利润率符合预期的几款产品，然后花一点时间完成每一款产品的词库，竞品库，用户画像，产品文案的搭建，至于产品的视觉，弄个简易的初稿就成，先创建一下链接，售价定的高一点，投放下广告做下测试。

跨境电商长期主义 2022-02-18 销售第一步：搞明白谁是你的用户: # 销售第一步：搞明白谁是你的用户
作者：跨境电商长期主义
发布时间：2022-02-18 00:00:00
原文链接：https://mp.weixin.qq.com/s/persona2
来源文件：跨境电商长期主义html/persona2.html
通常面对咨询我的卖家朋友我都喜欢先问一个问题：你的目标客户到底是谁。要搞明白目标客户的精准画像，诸如兴趣偏好，活动领域以及消费价值，这才有可能服务好他们。`;

const STRUCTURED_RESULT = {
  data: {
    context: {
      chunks: [
        {
          content: `# 你是如何解决转化率的？
作者：跨境电商长期主义
发布时间：2022-06-18 08:50:45
原文链接：https://mp.weixin.qq.com/s/example6
来源文件：跨境电商长期主义html/example.html
产品首图极大程度上决定了点击率，但是如果没有点击率，转化率就成了一个没有必要的数据指标。
进入商品页面之后的提升，一方面靠文案和视觉，还有很重要的一点就是对比。`,
          metadata: {
            category: "跨境电商长期主义",
            title: "你是如何解决转化率的？",
          },
        },
      ],
    },
  },
};

const WORKFLOW_MEMORY_RESULT = {
  data: {
    context: {
      chunks: [
        {
          content: `# 亚马逊学习档案：视觉转化诊断

## 问题
主图视觉点击率转化率怎么优化？

## 当前结论
先把主图当成点击率入口处理；没有点击率，后面的转化率分析意义会变弱。

## 下一步
先看主图点击率，再看副图、五点和 A+。`,
          document_id: "1779678895_387644f2",
          score: 0.78,
          metadata: {
            namespace: "amazon-learning-workflow",
            key: "dossier/amazon-20260525-24bb81aa1ac4",
            source_type: "amazon-learning-dossier",
            title: "亚马逊学习档案：视觉转化诊断",
            score_breakdown: {
              final_score: 0.78,
              vector_similarity: 0.66,
            },
          },
        },
      ],
    },
  },
};

const BUSINESS_WORKFLOW_MEMORY_RESULT = {
  data: {
    context: {
      chunks: [
        {
          content: `# 亚马逊学习档案：主图材料复盘

## 用户业务材料
主图是白底图，CTR 0.25%，CVR 5%，SP 广告 ACOS 45%，核心关键词 garlic press。

## 下一步
先改主图并记录 CTR、CVR 和 ACOS 前后变化。`,
          metadata: {
            namespace: "amazon-learning-workflow",
            key: "dossier/business-material",
            source_type: "amazon-learning-dossier",
            title: "亚马逊学习档案：主图材料复盘",
          },
        },
      ],
    },
  },
};

const UNSTRUCTURED_BUSINESS_WORKFLOW_MEMORY_RESULT = {
  data: {
    context: {
      chunks: [
        {
          content: `# 亚马逊学习档案：主图记录

主图白底图，CTR 0.25%，CVR 5%，ACOS 45%，garlic press，先改主图再看广告。`,
          metadata: {
            namespace: "amazon-learning-workflow",
            key: "dossier/unstructured-business-material",
            source_type: "amazon-learning-dossier",
            title: "亚马逊学习档案：主图记录",
          },
        },
      ],
    },
  },
};

const EXPERIMENT_WORKFLOW_MEMORY_RESULT = {
  data: {
    context: {
      chunks: [
        {
          content: `# 亚马逊学习档案：主图实验复盘

## 实验复盘
主图 A/B 小实验 7 天，CTR 从 0.25% 到 0.42%，CVR 从 5% 到 5.2%，ACOS 从 45% 到 38%。

## 下一步
继续复核 7 天，避免只看单次波动。`,
          metadata: {
            namespace: "amazon-learning-workflow",
            key: "dossier/experiment-review",
            source_type: "amazon-learning-dossier",
            title: "亚马逊学习档案：主图实验复盘",
          },
        },
      ],
    },
  },
};

const NEUTRAL_WORKFLOW_MEMORY_RESULT = {
  data: {
    context: {
      chunks: [
        {
          content: `# 亚马逊学习档案：资料整理

## 当前结论
把主图、Listing、广告和关键词分成四个模块复盘，逐项记录还缺哪些材料。`,
          metadata: {
            namespace: "amazon-learning-workflow",
            key: "dossier/neutral",
            source_type: "amazon-learning-dossier",
            title: "亚马逊学习档案：资料整理",
          },
        },
      ],
    },
  },
};

const CAUTION_SOURCE_CONTEXT = `Query: 主图视觉点击率转化率怎么优化？

张子卿 2026-04-02 不要先改主图: # 不要先改主图
作者：张子卿
发布时间：2026-04-02 08:00:00
原文链接：https://mp.weixin.qq.com/s/memory-conflict-source
来源文件：张子卿html/memory-conflict-source.html
转化率问题不建议先改主图，主图不是当前瓶颈，应该先看评价、价格和页面承接。`;

test("parseOpenHumanContext extracts retrieved articles with metadata", () => {
  const articles = parseOpenHumanContext(SAMPLE_CONTEXT);

  assert.equal(articles.length, 2);
  assert.equal(articles[0].author, "飞翔的波波");
  assert.equal(articles[0].date, "2025-09-15");
  assert.equal(articles[0].sourceUrl, "https://mp.weixin.qq.com/s/example1");
  assert.match(articles[0].excerpt, /市场容量决定了销量的天花板/);
  assert.equal(articles[1].author, "跨境电商长期主义");
});

test("parseOpenHumanContext accepts user-added sources as source material", () => {
  const articles = parseOpenHumanContext(`# 我的竞品调研
作者：我的资料
发布时间：2026-05-31
原文链接：user-source://user-test
来源文件：user-sources/user-test.json
紫星指标低于 3 时，先检查主图首屏利益点，不要先调广告。`);

  assert.equal(articles.length, 1);
  assert.equal(articles[0].author, "我的资料");
  assert.equal(articles[0].title, "我的竞品调研");
  assert.equal(articles[0].sourceType, "user_material");
  assert.match(articles[0].body, /紫星指标低于 3/);
});

test("buildQaPayload keeps user-added sources out of author original evidence", () => {
  const payload = buildQaPayload("主图首屏利益点先看什么？", `# 我的竞品调研
作者：我的资料
发布时间：2026-05-31
原文链接：user-source://user-test
来源文件：user-sources/user-test.json
紫星指标低于 3 时，先检查主图首屏利益点，不要先调广告。`, "主图首屏利益点先看什么？");

  assert.equal(payload.sources.length, 1);
  assert.equal(payload.sources[0].author, "我的资料");
  assert.equal(payload.sources[0].sourceType, "user_material");
  assert.ok(payload.evidenceChain.claims.some((claim) => claim.type === "user_material"));
  assert.equal(payload.evidenceChain.claims.some((claim) => claim.type === "source_evidence"), false);
  assert.equal(payload.sourceTrust.status, "needs_source");
  assert.match(payload.sourceTrust.summary, /没有可定位的作者原文证据/);
});

test("scoreSentences prioritizes sentences that match the question", () => {
  const articles = parseOpenHumanContext(SAMPLE_CONTEXT);
  const ranked = scoreSentences("新品选品市场容量怎么判断", articles, 3);

  assert.ok(ranked.length >= 2);
  assert.match(ranked[0].text, /市场容量|核心关键词|月销量/);
  assert.equal(ranked[0].source.author, "飞翔的波波");
});

test("scoreSentences avoids generic judgment sentences when domain terms exist", () => {
  const noisyContext = `${SAMPLE_CONTEXT}

张子卿 2024-11-14 亚马逊实盘记录45 —— 平行兼容＞向上社交: # 亚马逊实盘记录45 —— 平行兼容＞向上社交
作者：张子卿
发布时间：2024-11-14 17:35:37
原文链接：https://mp.weixin.qq.com/s/example3
来源文件：张子卿html/example.html
我如何判断一个人是否值得交往，一般只看他是不是独立自强有自己的价值观和思考方式。`;
  const articles = parseOpenHumanContext(noisyContext);
  const ranked = scoreSentences("新品选品应该如何判断是否值得做？", articles, 3);

  assert.doesNotMatch(ranked.map((item) => item.text).join("\n"), /判断一个人是否值得交往/);
  assert.match(ranked[0].text, /市场|关键词|月销量|产品/);
});

test("buildQaPayload refuses weakly related retrieved articles as source evidence", () => {
  const weakContext = `Query: 主图视觉点击率转化率怎么优化？

张子卿 2024-11-14 亚马逊实盘记录45 —— 平行兼容＞向上社交: # 亚马逊实盘记录45 —— 平行兼容＞向上社交
作者：张子卿
发布时间：2024-11-14 17:35:37
原文链接：https://mp.weixin.qq.com/s/weak
来源文件：张子卿html/weak.html
很多人问主图这个事情，但这篇文章主要是在复盘个人状态、团队协作和长期写作节奏，不讨论点击率、转化率、Listing 或亚马逊页面优化。`;
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", weakContext);

  assert.equal(payload.sources.length, 0);
  assert.equal(payload.evidenceChain.claims[0].type, "needs_source");
  assert.equal(payload.validationPack.status, "needs_source");
  assert.equal(payload.sourceStudyPack.status, "needs_source");
  assert.match(payload.answer, /缺少来源/);
  assert.doesNotMatch(payload.answer, /我从本地知识库里找到了/);
});

test("buildQaPayload refuses out-of-domain semantic retrieval", () => {
  const payload = buildQaPayload("量子芯片低温纠错架构怎么搭？", MAIN_IMAGE_CONTEXT);

  assert.equal(payload.sources.length, 0);
  assert.equal(payload.validationPack.status, "needs_source");
  assert.equal(payload.evidenceChain.claims[0].type, "needs_source");
  assert.match(payload.answer, /缺少来源/);
});

test("buildQaPayload returns a readable answer and source list", () => {
  const payload = buildQaPayload("新品选品应该如何判断是否值得做？", SAMPLE_CONTEXT);

  assert.equal(payload.question, "新品选品应该如何判断是否值得做？");
  assert.ok(payload.answer.includes("资料里最相关的判断"));
  assert.ok(payload.answer.includes("飞翔的波波"));
  assert.ok(payload.sources.length >= 1);
  assert.equal(payload.sources[0].title, "从0到1判断产品能不能做——亚马逊市场分析全攻略");
  assert.ok(payload.suggestedQuestions.some((item) => item.includes("广告")));
});

test("normalizeContextText accepts OpenHuman wrapper objects", () => {
  const text = normalizeContextText({ logs: ["ok"], result: SAMPLE_CONTEXT });

  assert.match(text, /从0到1判断产品能不能做/);
  assert.doesNotMatch(text, /\[object Object\]/);
});

test("buildQaPayload accepts structured OpenHuman query results", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", STRUCTURED_RESULT);

  assert.match(payload.answer, /资料里最相关的判断/);
  assert.match(payload.answer, /点击率/);
  assert.equal(payload.sources[0].author, "跨境电商长期主义");
  assert.equal(payload.sources[0].sourceUrl, "https://mp.weixin.qq.com/s/example6");
});

test("normalizeLearningMemoryReminder extracts workflow memory without source semantics", () => {
  const reminder = normalizeLearningMemoryReminder(WORKFLOW_MEMORY_RESULT);

  assert.equal(reminder.items.length, 1);
  assert.equal(reminder.items[0].namespace, "amazon-learning-workflow");
  assert.equal(reminder.items[0].key, "dossier/amazon-20260525-24bb81aa1ac4");
  assert.match(reminder.items[0].title, /视觉转化诊断/);
  assert.match(reminder.items[0].excerpt, /先把主图当成点击率入口处理/);
  assert.match(reminder.boundary, /不是作者原文证据/);
});

test("normalizeLearningMemoryReminder labels business material and experiment history separately", () => {
  const businessReminder = normalizeLearningMemoryReminder(BUSINESS_WORKFLOW_MEMORY_RESULT);
  const experimentReminder = normalizeLearningMemoryReminder(EXPERIMENT_WORKFLOW_MEMORY_RESULT);

  assert.equal(businessReminder.items[0].memoryKind, "business_material");
  assert.equal(businessReminder.items[0].memoryKindLabel, "历史业务材料");
  assert.match(businessReminder.items[0].excerpt, /主图是白底图/);
  assert.match(businessReminder.boundary, /历史业务材料和实验复盘/);

  assert.equal(experimentReminder.items[0].memoryKind, "experiment_review");
  assert.equal(experimentReminder.items[0].memoryKindLabel, "历史实验复盘");
  assert.match(experimentReminder.items[0].excerpt, /CTR 从 0.25% 到 0.42%/);
  assert.match(experimentReminder.boundary, /不能替代作者原文证据/);
});

test("normalizeLearningMemoryReminder treats unstructured metric records as business material", () => {
  const reminder = normalizeLearningMemoryReminder(UNSTRUCTURED_BUSINESS_WORKFLOW_MEMORY_RESULT);

  assert.equal(reminder.items[0].memoryKind, "business_material");
  assert.equal(reminder.items[0].memoryKindLabel, "历史业务材料");
  assert.match(reminder.items[0].excerpt, /CTR 0.25%/);
});

test("normalizeLearningMemoryReminder prioritizes accepted source evidence over generic business boundary text", () => {
  const reminder = normalizeLearningMemoryReminder({
    data: {
      context: {
        chunks: [
          {
            content: `# 亚马逊学习档案：主图证据复盘

## 当前结论
先把主图当成点击率入口处理。

## 已采纳原文证据
- 跨境电商长期主义《你是如何解决转化率的？》：产品首图极大程度上决定了点击率。

## 用户业务材料
用户业务材料不是作者原文证据，后续只用于业务验证。`,
            document_id: "source-note",
            metadata: {
              namespace: "amazon-learning-workflow",
              key: "dossier/source-note",
              source_type: "amazon-learning-dossier",
              title: "亚马逊学习档案：主图证据复盘",
            },
          },
        ],
      },
    },
  });

  assert.equal(reminder.items[0].memoryKind, "source_evidence_note");
  assert.equal(reminder.items[0].memoryKindLabel, "历史证据笔记");
  assert.match(reminder.boundary, /不是作者原文证据/);
});

test("buildQaPayload shows workflow memory reminder while keeping citations source-only", () => {
  const payload = buildQaPayload("那我应该先改哪一块？", MAIN_IMAGE_CONTEXT, "主图视觉点击率转化率怎么优化？\n那我应该先改哪一块？", {
    learningMemoryContext: WORKFLOW_MEMORY_RESULT,
  });

  assert.equal(payload.learningMemoryReminder.items.length, 1);
  assert.match(payload.answer, /本地学习档案提醒/);
  assert.match(payload.answer, /不是作者原文证据/);
  assert.ok(payload.sources.length >= 1);
  assert.ok(payload.sources.every((source) => source.author === "跨境电商长期主义"));
  assert.ok(payload.rankedEvidence.every((item) => Number.isInteger(item.sourceIndex)));
  assert.ok(payload.evidenceChain.claims.every((claim) => claim.type !== "source_evidence" || claim.author === "跨境电商长期主义"));
  assert.ok(!payload.sources.some((source) => /学习档案|amazon-learning-workflow/.test(`${source.title} ${source.sourcePath}`)));
});

test("buildQaPayload keeps no-source boundary when only workflow memory is available", () => {
  const payload = buildQaPayload("那我应该先改哪一块？", "", "主图视觉点击率转化率怎么优化？\n那我应该先改哪一块？", {
    learningMemoryContext: WORKFLOW_MEMORY_RESULT,
  });

  assert.equal(payload.sources.length, 0);
  assert.equal(payload.evidenceChain.claims[0].type, "needs_source");
  assert.equal(payload.learningMemoryReminder.items.length, 1);
  assert.match(payload.answer, /本地学习档案提醒/);
  assert.match(payload.answer, /缺少来源/);
  assert.match(payload.answer, /不是作者原文证据/);
  assert.equal(payload.learningMemoryReminder.alignment.status, "needs_source");
  assert.match(payload.learningMemoryReminder.alignment.message, /不能替代作者原文证据/);
});

test("buildQaPayload does not turn workflow chunks into author sources", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", WORKFLOW_MEMORY_RESULT);

  assert.equal(payload.sources.length, 0);
  assert.equal(payload.evidenceChain.claims[0].type, "needs_source");
  assert.doesNotMatch(payload.answer, /我从本地知识库里找到了 1 篇相关资料/);
});

test("buildQaPayload keeps business memory out of source alignment conflicts", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", CAUTION_SOURCE_CONTEXT, "主图视觉点击率转化率怎么优化？", {
    learningMemoryContext: BUSINESS_WORKFLOW_MEMORY_RESULT,
  });

  assert.equal(payload.learningMemoryReminder.items[0].memoryKind, "business_material");
  assert.equal(payload.learningMemoryReminder.alignment.status, "neutral");
  assert.match(payload.answer, /历史业务材料/);
  assert.match(payload.answer, /只用于验证你的业务/);
  assert.doesNotMatch(payload.answer, /历史档案与本轮作者证据不一致/);
});

test("buildQaPayload warns when learning memory conflicts with current author evidence", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", CAUTION_SOURCE_CONTEXT, "主图视觉点击率转化率怎么优化？", {
    learningMemoryContext: WORKFLOW_MEMORY_RESULT,
  });

  assert.equal(payload.sources.length, 1);
  assert.equal(payload.sources[0].author, "张子卿");
  assert.equal(payload.evidenceAudit.counts.conflictSignals, 0);
  assert.equal(payload.learningMemoryReminder.alignment.status, "conflict");
  assert.match(payload.learningMemoryReminder.alignment.message, /历史档案与本轮作者证据不一致/);
  assert.ok(payload.learningMemoryReminder.alignment.conflicts.some((item) => item.concept === "主图"));
  assert.match(payload.answer, /历史档案与本轮作者证据不一致/);
  assert.match(payload.answer, /先以本轮作者证据为准/);
  assert.ok(!payload.sources.some((source) => /学习档案|amazon-learning-workflow/.test(`${source.title} ${source.sourcePath}`)));
});

test("buildQaPayload does not recommend first changing main image when the only source says not to", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", CAUTION_SOURCE_CONTEXT);

  assert.equal(payload.sources.length, 1);
  assert.match(payload.answer, /不建议先改主图/);
  assert.match(payload.answer, /先核对|暂不能判断/);
  assert.doesNotMatch(payload.answer, /先把主图当成点击率入口处理/);
  assert.doesNotMatch(payload.answer, /先看主图点击率/);
  assert.ok(payload.validationPack.dataRequests.some((item) => /点击率|CTR|评价|价格|页面/.test(`${item.label}${item.why}`)));
});

test("buildQaPayload does not treat do not adjust ads first as anti-main-image evidence", () => {
  const context = `# 我的竞品调研
作者：我的资料
发布时间：2026-05-31
原文链接：user-source://user-test
来源文件：user-sources/user-test.json
紫星指标低于 3 时，先检查主图首屏利益点，不要先调广告竞价。`;
  const payload = buildQaPayload("紫星指标低于 3 时，主图点击率应该先改哪里？", context, "紫星指标低于 3 时，主图点击率应该先改哪里？");

  assert.doesNotMatch(payload.answer, /不建议先改主图/);
  assert.match(payload.answer, /主图|首屏利益点/);
});

test("buildQaPayload can answer a generic follow-up from selected user source context", () => {
  const context = `# 我的竞品调研
作者：我的资料
发布时间：2026-05-31
原文链接：user-source://note-user-test
来源文件：user-sources/note-user-test.json
笔记星标：我自己的学习判断是先检查主图首屏利益点，再检查标题关键词。`;
  const payload = buildQaPayload("我下一步先做什么？", context, `我下一步先做什么？\n所选我的资料：\n${context}`);

  assert.equal(payload.sources.length, 1);
  assert.equal(payload.sources[0].author, "我的资料");
  assert.match(payload.answer, /主图|首屏利益点|关键词/);
});

test("buildQaPayload constrains product diagnosis when current source rejects first changing main image", () => {
  const question = [
    "请结合这些数据判断我应该先改哪一块：",
    "主图是白底图，和竞品差不多",
    "CTR 0.25%，CVR 5%，最近 7 天 session 1200",
  ].join("\n");
  const payload = buildQaPayload(question, CAUTION_SOURCE_CONTEXT, question, {
    productInput: { text: question },
  });

  assert.match(payload.answer, /不建议先改主图/);
  assert.match(payload.answer, /先核对|暂不能判断/);
  assert.doesNotMatch(payload.answer, /最先检查：先改搜索结果里的主图点击入口/);
  assert.doesNotMatch(payload.answer, /判断：当前优先改主图点击入口/);
});

test("buildQaPayload marks learning memory aligned only as a reminder", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT, "主图视觉点击率转化率怎么优化？", {
    learningMemoryContext: WORKFLOW_MEMORY_RESULT,
  });

  assert.equal(payload.learningMemoryReminder.alignment.status, "aligned");
  assert.match(payload.learningMemoryReminder.alignment.message, /方向一致/);
  assert.match(payload.learningMemoryReminder.boundary, /不是作者原文证据/);
});

test("buildQaPayload keeps neutral learning memory from becoming a false conflict", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", CAUTION_SOURCE_CONTEXT, "主图视觉点击率转化率怎么优化？", {
    learningMemoryContext: NEUTRAL_WORKFLOW_MEMORY_RESULT,
  });

  assert.equal(payload.learningMemoryReminder.alignment.status, "neutral");
  assert.equal(payload.learningMemoryReminder.alignment.conflicts.length, 0);
  assert.match(payload.learningMemoryReminder.alignment.message, /没有形成明确一致或冲突关系/);
  assert.doesNotMatch(payload.answer, /历史档案与本轮作者证据不一致/);
});

test("normalizeContextText preserves source url and path from structured chunks", () => {
  const context = normalizeContextText({
    data: {
      context: {
        chunks: [
          {
            content: "产品首图极大程度上决定了点击率。",
            metadata: {
              category: "跨境电商长期主义",
              title: "你是如何解决转化率的？",
              published_at: "2022-06-18T08:50:45Z",
              source_url: "https://mp.weixin.qq.com/s/source-url",
              source_path: "跨境电商长期主义html/source.html",
            },
          },
        ],
      },
    },
  });
  const articles = parseOpenHumanContext(context);

  assert.equal(articles[0].sourceUrl, "https://mp.weixin.qq.com/s/source-url");
  assert.equal(articles[0].sourcePath, "跨境电商长期主义html/source.html");
});

test("buildQaPayload prioritizes sources used by the answer", () => {
  const noisyContext = `${SAMPLE_CONTEXT}

张子卿 2024-11-14 亚马逊实盘记录45 —— 平行兼容＞向上社交: # 亚马逊实盘记录45 —— 平行兼容＞向上社交
作者：张子卿
发布时间：2024-11-14 17:35:37
原文链接：https://mp.weixin.qq.com/s/example3
来源文件：张子卿html/example.html
我如何判断一个人是否值得交往，一般只看他是不是独立自强有自己的价值观和思考方式。`;
  const payload = buildQaPayload("新品选品应该如何判断是否值得做？", noisyContext);

  assert.notEqual(payload.sources[0].title, "亚马逊实盘记录45 —— 平行兼容＞向上社交");
  assert.ok(!payload.sources.some((source) => source.title.includes("平行兼容")));
  assert.ok(payload.sources.some((source) => source.title.includes("从0到1判断产品能不能做")));
});

test("buildQaPayload excludes user-blocked sources from answer evidence", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT, "主图视觉点击率转化率怎么优化？", {
    excludedSourceKeys: ["跨境电商长期主义|2026-03-25|很多亚马逊上的产品，压根不具备接客资格。"],
  });

  assert.ok(payload.sources.length >= 1);
  assert.ok(!payload.sources.some((source) => source.title.includes("接客资格")));
  assert.ok(payload.evidenceChain.claims.every((claim) => claim.title !== "很多亚马逊上的产品，压根不具备接客资格。"));
  assert.doesNotMatch(payload.answer, /主图是唯一一个纯视觉元素/);
});

test("buildQaPayload limits answer evidence to allowed authors", () => {
  const payload = buildQaPayload("新品选品应该如何判断是否值得做？", SAMPLE_CONTEXT, "新品选品应该如何判断是否值得做？", {
    allowedAuthors: ["飞翔的波波"],
  });

  assert.ok(payload.sources.length >= 1);
  assert.ok(payload.sources.every((source) => source.author === "飞翔的波波"));
  assert.ok(payload.evidenceChain.claims.every((claim) => !claim.author || claim.author === "飞翔的波波"));
  assert.ok(payload.graph.nodes.every((node) => node.type !== "author" || node.label === "飞翔的波波"));
  assert.ok(payload.learningCard.evidence.every((item) => item.author === "飞翔的波波"));
  assert.equal(payload.sourceScope.active, true);
  assert.deepEqual(payload.sourceScope.allowedAuthors, ["飞翔的波波"]);
  assert.match(payload.sourceScope.summary, /飞翔的波波/);
});

test("buildQaPayload limits answer evidence to explicitly selected source keys", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT, "主图视觉点击率转化率怎么优化？", {
    allowedSourceKeys: ["跨境电商长期主义|2022-06-18|你是如何解决转化率的？"],
  });

  assert.equal(payload.sources.length, 1);
  assert.equal(payload.sources[0].title, "你是如何解决转化率的？");
  assert.ok(payload.evidenceChain.claims.every((claim) => claim.type !== "source_evidence" || claim.title === "你是如何解决转化率的？"));
  assert.equal(payload.sourceScope.active, true);
  assert.deepEqual(payload.sourceScope.allowedSourceKeys, ["跨境电商长期主义|2022-06-18|你是如何解决转化率的？"]);
  assert.match(payload.sourceScope.summary, /已选择的 1 个来源/);
});

test("buildQaPayload does not fall back to other sources when selected source keys miss", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT, "主图视觉点击率转化率怎么优化？", {
    allowedSourceKeys: ["不存在的来源"],
  });

  assert.equal(payload.sources.length, 0);
  assert.equal(payload.evidenceChain.claims[0].type, "needs_source");
  assert.equal(payload.sourceScope.totalAfterScope, 0);
  assert.match(payload.answer, /缺少来源/);
});

test("buildQaPayload does not fall back to all authors when scoped authors have no sources", () => {
  const payload = buildQaPayload("新品选品应该如何判断是否值得做？", SAMPLE_CONTEXT, "新品选品应该如何判断是否值得做？", {
    allowedAuthors: ["不存在作者"],
  });

  assert.equal(payload.sources.length, 0);
  assert.equal(payload.evidenceChain.claims[0].type, "needs_source");
  assert.equal(payload.sourceScope.active, true);
  assert.deepEqual(payload.sourceScope.allowedAuthors, ["不存在作者"]);
  assert.equal(payload.sourceScope.totalAfterScope, 0);
  assert.match(payload.answer, /缺少来源/);
});

test("buildQaPayload combines allowed authors with excluded sources", () => {
  const payload = buildQaPayload("新品选品应该如何判断是否值得做？", SAMPLE_CONTEXT, "新品选品应该如何判断是否值得做？", {
    allowedAuthors: ["飞翔的波波"],
    excludedSourceKeys: ["飞翔的波波|2025-09-15|从0到1判断产品能不能做——亚马逊市场分析全攻略"],
  });

  assert.equal(payload.sources.length, 0);
  assert.equal(payload.evidenceChain.claims[0].type, "needs_source");
  assert.equal(payload.sourceScope.totalRetrieved, 2);
  assert.equal(payload.sourceScope.totalAfterScope, 0);
});

test("buildQaPayload includes product input summary without turning user input into source evidence", () => {
  const question = [
    "我补充了以下产品信息，请结合当前学习档案、已采纳证据和已排除来源，判断我现在应该先改哪一块：",
    "",
    "主图/视觉：",
    "1. 主图是白底图，和竞品差不多",
    "",
    "点击率/转化率数据：",
    "1. CTR 0.25%，CVR 5%，最近 7 天 session 1200",
    "",
    "广告/流量：",
    "1. SP 广告 ACOS 45%，预算每天 20 美金",
    "",
    "关键词/竞品：",
    "1. 核心关键词 garlic press，竞品 ASIN B001234567",
  ].join("\n");
  const payload = buildQaPayload(question, MAIN_IMAGE_CONTEXT, question, {
    productInput: {
      text: question,
    },
  });

  assert.ok(payload.productInputSummary);
  assert.match(payload.productInputSummary.summary, /用户产品信息/);
  assert.ok(payload.productInputSummary.facts.some((section) => section.label.includes("主图")));
  assert.ok(payload.productInputSummary.facts.some((section) => section.label.includes("点击率")));
  assert.ok(payload.productInputSummary.facts.some((section) => section.label.includes("广告")));
  assert.match(payload.productInputSummary.caution, /不是本地资料证据/);
  assert.ok(payload.diagnosisPanel);
  assert.ok(payload.diagnosisPanel.tracks.some((track) => track.label.includes("主图入口")));
  assert.ok(payload.diagnosisPanel.tracks.some((track) => track.label.includes("广告浪费")));
  assert.ok(
    payload.evidenceChain.claims.every(
      (claim) => claim.type !== "source_evidence" || !/CTR 0.25|CVR 5|ACOS 45|garlic press|B001234567/.test(claim.quote || claim.text || ""),
    ),
  );
});

test("buildQaPayload writes product-aware diagnosis priority into the answer", () => {
  const question = [
    "我补充了以下产品信息，请结合当前学习档案判断我现在应该先改哪一块：",
    "主图是白底图，和竞品差不多",
    "CTR 0.25%，CVR 5%，最近 7 天 session 1200",
    "SP 广告 ACOS 45%，预算每天 20 美金",
    "核心关键词 garlic press，竞品 ASIN B001234567",
  ].join("\n");
  const payload = buildQaPayload(question, MAIN_IMAGE_CONTEXT, question, {
    productInput: { text: question },
  });

  assert.match(payload.answer, /本轮诊断优先级/);
  assert.match(payload.answer, /最先检查：先改搜索结果里的主图点击入口/);
  assert.match(payload.answer, /CTR 0.25%/);
  assert.match(payload.answer, /CVR 5%/);
  assert.match(payload.answer, /garlic press/);
  assert.match(payload.answer, /B001234567/);
  assert.match(payload.answer, /用户输入不是原文证据/);
  assert.ok(
    payload.evidenceChain.claims.every(
      (claim) => claim.type !== "source_evidence" || !/CTR 0.25|CVR 5|ACOS 45|garlic press|B001234567/.test(claim.quote || claim.text || ""),
    ),
  );
});

test("buildQaPayload creates a business validation pack from source-backed visual answers", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);

  assert.ok(payload.validationPack);
  assert.equal(payload.validationPack.status, "source_backed");
  assert.match(payload.validationPack.boundary, /作者原文证据/);
  assert.ok(payload.validationPack.hypotheses.some((item) => Number.isInteger(item.sourceIndex) && /主图|点击率/.test(item.label)));
  assert.ok(payload.validationPack.dataRequests.some((item) => /点击率|CTR/.test(item.label)));
  assert.ok(payload.validationPack.experiments.some((item) => /主图|视觉/.test(item.title)));
  assert.ok(payload.validationPack.decisionRules.some((item) => /CTR|点击率|转化率/.test(`${item.if}${item.then}`)));
  assert.match(payload.validationPack.followUpPrompt, /我补充了验证数据/);
});

test("buildQaPayload keeps validation pack conservative when only product data exists", () => {
  const question = [
    "请结合这些数据判断我应该先改哪一块：",
    "主图是白底图，和竞品差不多",
    "CTR 0.25%，CVR 5%，最近 7 天 session 1200",
  ].join("\n");
  const payload = buildQaPayload(question, "", question, {
    productInput: { text: question },
  });

  assert.ok(payload.validationPack);
  assert.equal(payload.validationPack.status, "needs_source");
  assert.equal(payload.validationPack.hypotheses.length, 0);
  assert.ok(payload.validationPack.dataRequests.some((item) => /点击率|转化率|CTR|CVR/.test(item.label)));
  assert.match(payload.validationPack.boundary, /不能替代作者原文证据/);
  assert.doesNotMatch(JSON.stringify(payload.validationPack), /amazon-learning-workflow/);
});

test("buildQaPayload keeps separate CTR and CVR values from structured data fields", () => {
  const question = [
    "请结合这些数据判断我应该先改哪一块？",
    "",
    "我补充的产品数据：",
    "当前点击率和转化率：CTR 0.25%",
    "CVR 5.1%",
    "核心关键词或竞品链接：核心词 garlic press",
  ].join("\n");
  const payload = buildQaPayload(question, MAIN_IMAGE_CONTEXT, question, {
    productInput: { text: question },
  });

  assert.match(payload.answer, /CTR 0.25%/);
  assert.match(payload.answer, /CVR 5.1%/);
  assert.doesNotMatch(payload.answer, /CVR 0.25%/);
});

test("buildQaPayload reads structured product sections by stable ids", () => {
  const question = "请结合结构化数据判断我应该先改哪一块？";
  const payload = buildQaPayload(question, MAIN_IMAGE_CONTEXT, question, {
    productInput: {
      sections: [
        { id: "visual", items: ["主图白底图，和竞品差不多"], missing: [] },
        { id: "metrics", items: ["CTR 0.25%", "CVR 5%"], missing: [] },
      ],
    },
  });

  assert.match(payload.answer, /最先检查：先改搜索结果里的主图点击入口/);
  assert.match(payload.answer, /CTR 0.25%/);
  assert.match(payload.answer, /CVR 5%/);
  assert.ok(payload.productInputSummary.facts.some((section) => section.label.includes("主图")));
  assert.ok(payload.productInputSummary.facts.some((section) => section.label.includes("点击率")));
});

test("buildQaPayload pairs Chinese CTR and CVR values by metric order", () => {
  const question = [
    "请结合这些数据判断我应该先改哪一块？",
    "主图是白底图，和竞品差不多",
    "转化率和点击率分别是 5% 和 0.25%",
  ].join("\n");
  const payload = buildQaPayload(question, MAIN_IMAGE_CONTEXT, question, {
    productInput: { text: question },
  });

  assert.match(payload.answer, /CTR 0.25%/);
  assert.match(payload.answer, /CVR 5%/);
  assert.doesNotMatch(payload.answer, /CTR 5%/);
  assert.doesNotMatch(payload.answer, /CVR 0.25%/);
});

test("buildQaPayload keeps ACOS visible when ad metrics share one line with CTR and CVR", () => {
  const question = [
    "请结合这些数据判断我应该先改哪一块？",
    "主图是白底图，和竞品差不多",
    "广告数据：CTR 0.3% CVR 4% ACOS 42%",
  ].join("\n");
  const payload = buildQaPayload(question, MAIN_IMAGE_CONTEXT, question, {
    productInput: { text: question },
  });

  assert.match(payload.answer, /ACOS 42%/);
  assert.ok(payload.diagnosisPanel.tracks.some((track) => track.label.includes("广告浪费") && track.level === "高风险"));
});

test("buildQaPayload still gives product diagnosis when no source is found", () => {
  const question = [
    "我补充了以下产品信息，请判断先改哪一块：",
    "主图是白底图，和竞品差不多",
    "CTR 0.25%，CVR 5%，最近 7 天 session 1200",
    "核心关键词 garlic press，竞品 ASIN B001234567",
  ].join("\n");
  const payload = buildQaPayload(question, "", question, {
    productInput: { text: question },
  });

  assert.match(payload.answer, /本轮诊断优先级/);
  assert.match(payload.answer, /最先检查：先改搜索结果里的主图点击入口/);
  assert.match(payload.answer, /【缺少来源】/);
  assert.match(payload.answer, /这次没有命中本地资料/);
  assert.doesNotMatch(payload.answer, /用户输入 \\+ 本地资料/);
  assert.equal(payload.evidenceChain.claims[0].type, "needs_source");
  assert.equal(payload.sources.length, 0);
  assert.match(payload.diagnosisPanel.caution, /这次没有命中本地资料/);
  assert.ok(payload.diagnosisPanel.tracks.some((track) => track.label.includes("需要补充")));
});

test("buildProductInputSummary stays hidden for generic questions", () => {
  const summary = buildProductInputSummary({ question: "主图视觉点击率转化率怎么优化？" });

  assert.equal(summary, undefined);
});

test("buildQaPayload marks missing support when all retrieved sources are blocked", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT, "主图视觉点击率转化率怎么优化？", {
    excludedSourceKeys: [
      "跨境电商长期主义|2022-06-18|你是如何解决转化率的？",
      "跨境电商长期主义|2026-03-25|很多亚马逊上的产品，压根不具备接客资格。",
    ],
  });

  assert.equal(payload.sources.length, 0);
  assert.equal(payload.evidenceChain.claims[0].type, "needs_source");
  assert.match(payload.answer, /缺少来源/);
});

test("buildRetrievalQuery carries recent conversation into follow-up questions", () => {
  const query = buildRetrievalQuery("那应该先改哪一块？", [
    { role: "user", content: "主图视觉点击率转化率怎么优化？" },
    {
      role: "assistant",
      content: "先检查主图差异化、图片体系、文案视觉和流量来源。",
      evidenceChain: {
        claims: [
          {
            type: "source_evidence",
            quote: "产品首图极大程度上决定了点击率，但是如果没有点击率，转化率就成了一个没有必要的数据指标。",
            title: "你是如何解决转化率的？",
          },
        ],
      },
    },
  ]);

  assert.match(query, /当前问题：那应该先改哪一块？/);
  assert.match(query, /主图视觉点击率转化率怎么优化/);
  assert.match(query, /产品首图极大程度上决定了点击率/);
  assert.doesNotMatch(query, /先检查主图差异化/);
  assert.doesNotMatch(query, /文案视觉和流量来源/);
});

test("buildRetrievalQuery does not carry old visual context into a standalone new topic", () => {
  const query = buildRetrievalQuery("人群画像应该怎么构建？有哪些实操指导建议", [
    { role: "user", content: "主图视觉点击率转化率怎么优化？" },
    {
      role: "assistant",
      content: "先检查主图差异化、图片体系、文案视觉和流量来源。",
      evidenceChain: {
        claims: [
          {
            type: "source_evidence",
            quote: "产品首图极大程度上决定了点击率，但是如果没有点击率，转化率就成了一个没有必要的数据指标。",
            title: "你是如何解决转化率的？",
          },
        ],
      },
    },
  ]);

  assert.match(query, /^人群画像应该怎么构建？有哪些实操指导建议/);
  assert.match(query, /检索补充/);
  assert.match(query, /用户画像/);
  assert.match(query, /竞品信息/);
  assert.match(query, /搜索词/);
  assert.doesNotMatch(query, /主图视觉点击率转化率怎么优化/);
  assert.doesNotMatch(query, /产品首图极大程度上决定了点击率/);
});

test("buildRetrievalQuery excludes assistant inference and action advice from follow-up retrieval", () => {
  const query = buildRetrievalQuery("那应该先改哪一块？", [
    { role: "user", content: "主图视觉点击率转化率怎么优化？" },
    {
      role: "assistant",
      content: [
        "可执行结论：",
        "1. 先把主图当成点击率入口处理。 【推断1】",
        "执行顺序：",
        "1. 先看主图点击率 【行动1】",
      ].join("\n"),
      evidenceChain: {
        claims: [
          {
            type: "system_inference",
            text: "先把主图当成点击率入口处理。",
          },
          {
            type: "action_advice",
            text: "先看主图点击率",
          },
          {
            type: "source_evidence",
            quote: "主图是唯一一个纯视觉元素。",
            title: "很多亚马逊上的产品，压根不具备接客资格。",
          },
        ],
      },
    },
  ]);

  assert.match(query, /上轮已引用原文证据/);
  assert.match(query, /主图是唯一一个纯视觉元素/);
  assert.doesNotMatch(query, /先把主图当成点击率入口处理/);
  assert.doesNotMatch(query, /先看主图点击率/);
});

test("buildRetrievalQuery excludes restored no-source diagnosis snapshots from retrieval", () => {
  const query = buildRetrievalQuery("那我继续先看哪一块？", [
    { role: "user", content: "我这个产品应该先改哪一块？" },
    {
      role: "assistant",
      restoredFromDossierId: "diagnosis-dossier",
      content: [
        "已从学习档案恢复上下文。",
        "已采纳证据：暂无。",
        "已保存产品诊断：",
        "1. 先改搜索结果里的主图点击入口（已勾选 1/24 项）",
        "用户输入：CTR 0.25%，ACOS 45%。",
      ].join("\n"),
      evidenceChain: {
        claims: [
          {
            id: "needs-source:0",
            type: "needs_source",
            text: "这个学习档案没有保存可直接复用的原文证据。",
          },
        ],
      },
      diagnosisPanel: {
        priority: "先改搜索结果里的主图点击入口",
      },
    },
  ]);

  assert.match(query, /当前问题：那我继续先看哪一块？/);
  assert.match(query, /用户问题：我这个产品应该先改哪一块？/);
  assert.doesNotMatch(query, /已保存产品诊断/);
  assert.doesNotMatch(query, /先改搜索结果里的主图点击入口/);
  assert.doesNotMatch(query, /CTR 0.25%/);
  assert.doesNotMatch(query, /ACOS 45%/);
});

test("buildRetrievalQuery does not carry no-source assistant summaries into follow-up retrieval", () => {
  const query = buildRetrievalQuery("那我应该先改哪一块？", [
    { role: "user", content: "主图视觉点击率转化率怎么优化？" },
    {
      role: "assistant",
      content: "这次没有从本地知识库里找到足够相关的资料。建议先把主图当成点击率入口处理。",
      evidenceChain: {
        claims: [
          {
            id: "needs-source:0",
            type: "needs_source",
            text: "这次没有从本地资料里找到足够明确的原文证据。",
          },
        ],
      },
      learningMemoryReminder: {
        items: [{ title: "视觉转化诊断", excerpt: "先把主图当成点击率入口处理。" }],
      },
    },
  ]);

  assert.match(query, /用户问题：主图视觉点击率转化率怎么优化/);
  assert.doesNotMatch(query, /上轮回答摘要/);
  assert.doesNotMatch(query, /先把主图当成点击率入口处理/);
});

test("buildRetrievalQuery does not carry historical business memory into follow-up retrieval", () => {
  const query = buildRetrievalQuery("那下一步先改什么？", [
    { role: "user", content: "主图视觉点击率转化率怎么优化？" },
    {
      role: "assistant",
      content: [
        "本地学习档案提醒：",
        "1. [历史业务材料] 你之前沉淀过「主图材料复盘」：主图是白底图，CTR 0.25%，ACOS 45%。",
        "资料里最相关的判断：",
        "1. 产品首图极大程度上决定了点击率。（跨境电商长期主义《你是如何解决转化率的？》） 【证据1】",
      ].join("\n"),
      learningMemoryReminder: {
        items: [
          {
            title: "主图材料复盘",
            excerpt: "主图是白底图，CTR 0.25%，SP 广告 ACOS 45%，核心关键词 garlic press。",
            memoryKind: "business_material",
            memoryKindLabel: "历史业务材料",
          },
        ],
      },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            quote: "产品首图极大程度上决定了点击率。",
            title: "你是如何解决转化率的？",
          },
        ],
      },
    },
  ]);

  assert.match(query, /产品首图极大程度上决定了点击率/);
  assert.doesNotMatch(query, /主图是白底图/);
  assert.doesNotMatch(query, /CTR 0.25%/);
  assert.doesNotMatch(query, /ACOS 45%/);
  assert.doesNotMatch(query, /garlic press/);
});

test("buildRetrievalQuery redacts previous user business facts from follow-up retrieval", () => {
  const query = buildRetrievalQuery("那下一步先改什么？", [
    {
      role: "user",
      content: [
        "主图视觉点击率转化率怎么优化？",
        "主图是白底图，和竞品差不多",
        "CTR 0.25%，CVR 5%，SP 广告 ACOS 45%",
        "核心关键词 garlic press，竞品 ASIN B001234567",
      ].join("\n"),
    },
    {
      role: "assistant",
      content: "资料里最相关的判断。",
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            quote: "产品首图极大程度上决定了点击率。",
            title: "你是如何解决转化率的？",
          },
        ],
      },
    },
  ]);

  assert.match(query, /主图视觉点击率转化率怎么优化/);
  assert.match(query, /产品首图极大程度上决定了点击率/);
  assert.doesNotMatch(query, /白底图/);
  assert.doesNotMatch(query, /CTR 0.25%/);
  assert.doesNotMatch(query, /CVR 5%/);
  assert.doesNotMatch(query, /ACOS 45%/);
  assert.doesNotMatch(query, /garlic press/);
  assert.doesNotMatch(query, /B001234567/);
});

test("buildRetrievalQuery redacts current question business facts before source retrieval", () => {
  const query = buildRetrievalQuery([
    "请结合我补充的产品数据和本地作者资料，重新判断下一步优先级。",
    "原问题：主图视觉点击率转化率怎么优化？",
    "我补充的产品数据：",
    "- ASIN/类目：B001234567 garlic press",
    "- 曝光/点击率/转化率：CTR 0.25%，CVR 5%",
    "- 广告数据：ACOS 45%",
    "- Listing 或主图当前情况：白底图，和竞品差不多",
  ].join("\n"), []);

  assert.match(query, /主图视觉点击率转化率怎么优化/);
  assert.match(query, /补充的产品数据/);
  assert.doesNotMatch(query, /B001234567/);
  assert.doesNotMatch(query, /garlic press/);
  assert.doesNotMatch(query, /0.25/);
  assert.doesNotMatch(query, /5%/);
  assert.doesNotMatch(query, /45%/);
  assert.doesNotMatch(query, /白底图/);
});

test("buildRetrievalQuery redacts inline keyword business facts from prior questions", () => {
  const query = buildRetrievalQuery("那下一步先改什么？", [
    {
      role: "user",
      content: "主图视觉怎么优化？当前 ASIN B001234567，关键词 garlic press，CTR 0.25%。",
    },
  ]);

  assert.match(query, /主图视觉怎么优化/);
  assert.match(query, /关键词/);
  assert.match(query, /CTR/);
  assert.doesNotMatch(query, /CTR 0.25/);
  assert.doesNotMatch(query, /garlic press/);
  assert.doesNotMatch(query, /B001234567/);
  assert.doesNotMatch(query, /0.25/);
});

test("buildRetrievalQuery redacts natural metric and English keyword business facts", () => {
  const query = buildRetrievalQuery("那下一步先改什么？", [
    {
      role: "user",
      content: "主图视觉怎么优化？CTR 是 0.25%，点击率是 0.25%，ACOS 约 45%，CVR 为 5%，keyword garlic press，search term lemon squeezer。",
    },
  ]);

  assert.match(query, /主图视觉怎么优化/);
  assert.match(query, /CTR/);
  assert.match(query, /点击率/);
  assert.match(query, /ACOS/);
  assert.match(query, /CVR/);
  assert.doesNotMatch(query, /0.25/);
  assert.doesNotMatch(query, /45%/);
  assert.doesNotMatch(query, /5%/);
  assert.doesNotMatch(query, /garlic press/);
  assert.doesNotMatch(query, /lemon squeezer/);
});

test("buildRetrievalQuery redacts dashed and English metric business facts", () => {
  const query = buildRetrievalQuery("那下一步先改什么？", [
    {
      role: "user",
      content: "主图视觉怎么优化？CTR - 0.25%，click through rate is 0.25%，conversion rate is 5%，acos is 45%。",
    },
  ]);

  assert.match(query, /主图视觉怎么优化/);
  assert.match(query, /CTR/);
  assert.match(query, /click through rate/);
  assert.match(query, /conversion rate/);
  assert.match(query, /acos/i);
  assert.doesNotMatch(query, /0.25/);
  assert.doesNotMatch(query, /45%/);
  assert.doesNotMatch(query, /5%/);
});

test("buildRetrievalQuery prioritizes evidence the user marked useful", () => {
  const query = buildRetrievalQuery("那应该先改哪一块？", [
    { role: "user", content: "主图视觉点击率转化率怎么优化？" },
    {
      role: "assistant",
      content: "先检查主图差异化、图片体系、文案视觉和流量来源。",
      evidenceFeedback: {
        "source-evidence:0": "irrelevant",
        "source-evidence:1": "useful",
      },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            quote: "主图是唯一一个纯视觉元素。",
            title: "很多亚马逊上的产品，压根不具备接客资格。",
          },
          {
            id: "source-evidence:1",
            type: "source_evidence",
            quote: "产品首图极大程度上决定了点击率。",
            title: "你是如何解决转化率的？",
          },
          {
            id: "system-inference:0",
            type: "system_inference",
            text: "先把主图当成点击率入口处理。",
          },
        ],
      },
    },
  ]);

  assert.match(query, /用户标记有用的原文/);
  assert.match(query, /产品首图极大程度上决定了点击率/);
  assert.doesNotMatch(query, /主图是唯一一个纯视觉元素/);
  assert.doesNotMatch(query, /先把主图当成点击率入口处理/);
});

test("buildRetrievalQuery does not carry useful evidence from excluded sources", () => {
  const query = buildRetrievalQuery("那应该先改哪一块？", [
    { role: "user", content: "主图视觉点击率转化率怎么优化？" },
    {
      role: "assistant",
      content: "先检查主图差异化、图片体系、文案视觉和流量来源。",
      sources: [
        {
          author: "跨境电商长期主义",
          date: "2022-06-18",
          title: "你是如何解决转化率的？",
          sourcePath: "跨境电商长期主义html/example.html",
          excerpt: "产品首图极大程度上决定了点击率。",
        },
      ],
      evidenceFeedback: {
        "source-evidence:0": "useful",
      },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            quote: "产品首图极大程度上决定了点击率。",
            sourceIndex: 0,
            title: "你是如何解决转化率的？",
          },
        ],
      },
    },
  ], {
    excludedSourceKeys: ["跨境电商长期主义html/example.html"],
  });

  assert.match(query, /当前问题：那应该先改哪一块？/);
  assert.match(query, /用户问题：主图视觉点击率转化率怎么优化/);
  assert.doesNotMatch(query, /产品首图极大程度上决定了点击率/);
  assert.doesNotMatch(query, /你是如何解决转化率/);
});

test("buildRetrievalQuery excludes rejected evidence while keeping neutral evidence", () => {
  const query = buildRetrievalQuery("还有什么要检查？", [
    {
      role: "assistant",
      content: "先检查主图差异化、图片体系、文案视觉和流量来源。",
      evidenceFeedback: {
        "source-evidence:0": "irrelevant",
      },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            quote: "主图是唯一一个纯视觉元素。",
            title: "很多亚马逊上的产品，压根不具备接客资格。",
          },
          {
            id: "source-evidence:1",
            type: "source_evidence",
            quote: "进入商品页面之后的提升，一方面靠文案和视觉，还有很重要的一点就是对比。",
            title: "你是如何解决转化率的？",
          },
        ],
      },
    },
  ]);

  assert.match(query, /上轮已引用原文证据/);
  assert.match(query, /进入商品页面之后的提升/);
  assert.doesNotMatch(query, /主图是唯一一个纯视觉元素/);
});

test("buildRetrievalQuery skips prior evidence when whole answer needs retry", () => {
  const query = buildRetrievalQuery("那应该怎么重新判断？", [
    { role: "user", content: "主图视觉点击率转化率怎么优化？" },
    {
      role: "assistant",
      content: "先检查主图差异化、图片体系、文案视觉和流量来源。",
      evidenceAudit: {
        feedback: "retry",
      },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            quote: "产品首图极大程度上决定了点击率。",
            title: "你是如何解决转化率的？",
          },
        ],
      },
    },
  ]);

  assert.match(query, /用户问题：主图视觉点击率转化率怎么优化/);
  assert.doesNotMatch(query, /上轮已引用原文证据/);
  assert.doesNotMatch(query, /产品首图极大程度上决定了点击率/);
});

test("buildRetrievalQuery skips prior evidence when citation was marked wrong", () => {
  const query = buildRetrievalQuery("那应该怎么重新判断？", [
    {
      role: "assistant",
      content: "先检查主图差异化、图片体系、文案视觉和流量来源。",
      evidenceAudit: {
        feedback: "citation_wrong",
      },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            quote: "主图是唯一一个纯视觉元素。",
            title: "很多亚马逊上的产品，压根不具备接客资格。",
          },
        ],
      },
    },
  ]);

  assert.doesNotMatch(query, /上轮已引用原文证据/);
  assert.doesNotMatch(query, /主图是唯一一个纯视觉元素/);
});

test("buildRetrievalQuery uses answer effectiveness to request more sources", () => {
  const query = buildRetrievalQuery("继续帮我找来源", [
    { role: "user", content: "主图视觉点击率转化率怎么优化？" },
    {
      role: "assistant",
      content: "先检查主图差异化。",
      answerEffectiveness: {
        status: "needs_source",
        question: "主图视觉点击率转化率怎么优化？",
      },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            quote: "主图是唯一一个纯视觉元素。",
            title: "很多亚马逊上的产品，压根不具备接客资格。",
          },
        ],
      },
    },
  ]);

  assert.match(query, /用户确认上轮需要补来源/);
  assert.match(query, /优先重新检索作者原文证据/);
  assert.match(query, /原问题：主图视觉点击率转化率怎么优化/);
  assert.doesNotMatch(query, /上轮已引用原文证据/);
  assert.doesNotMatch(query, /主图是唯一一个纯视觉元素/);
  assert.doesNotMatch(query, /先检查主图差异化/);
});

test("buildRetrievalQuery uses answer effectiveness to switch intent without carrying assistant advice", () => {
  const query = buildRetrievalQuery("重新判断", [
    {
      role: "assistant",
      content: "你应该先改主图。",
      answerEffectiveness: {
        status: "switch_intent",
        question: "我是在学习方法还是诊断产品？",
      },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            quote: "产品首图极大程度上决定了点击率。",
            title: "你是如何解决转化率的？",
          },
        ],
      },
    },
  ]);

  assert.match(query, /用户确认上轮需要切换意图/);
  assert.match(query, /方法学习、产品诊断、实验复盘还是补来源检索/);
  assert.doesNotMatch(query, /你应该先改主图/);
  assert.doesNotMatch(query, /产品首图极大程度上决定了点击率/);
});

test("buildRetrievalQuery redacts product data from answer effectiveness prompts", () => {
  const query = buildRetrievalQuery("那下一步怎么判断？", [
    {
      role: "assistant",
      content: "需要补产品数据。",
      answerEffectiveness: {
        status: "add_product_data",
        question: "我的 ASIN B001234567，CTR 0.25%，核心关键词 garlic press，应该先改什么？",
      },
    },
  ]);

  assert.match(query, /用户确认上轮需要补产品数据/);
  assert.match(query, /产品细节要和作者原文证据分开处理/);
  assert.match(query, /主题：点击率、关键词/);
  assert.match(query, /点击率/);
  assert.doesNotMatch(query, /CTR 0.25/);
  assert.doesNotMatch(query, /我的产品/);
  assert.match(query, /关键词/);
  assert.doesNotMatch(query, /ASIN B001234567/);
  assert.doesNotMatch(query, /ASIN/);
  assert.doesNotMatch(query, /B001234567/);
  assert.doesNotMatch(query, /0.25/);
  assert.doesNotMatch(query, /garlic press/);
});

test("buildRetrievalQuery add product data skips old evidence and assistant advice", () => {
  const query = buildRetrievalQuery("补完数据后重新判断", [
    {
      role: "assistant",
      content: "你应该立刻先改主图。",
      answerEffectiveness: {
        status: "add_product_data",
        question: "我的 ASIN B001234567 主图 CTR 0.25%，应该先改什么？",
      },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            quote: "产品首图极大程度上决定了点击率。",
            title: "你是如何解决转化率的？",
          },
        ],
      },
    },
  ]);

  assert.match(query, /用户确认上轮需要补产品数据/);
  assert.match(query, /主题：主图、点击率/);
  assert.doesNotMatch(query, /你应该立刻先改主图/);
  assert.doesNotMatch(query, /产品首图极大程度上决定了点击率/);
  assert.doesNotMatch(query, /B001234567/);
  assert.doesNotMatch(query, /0.25/);
});

test("buildRetrievalQuery keeps confirmed effective answer as learning context with source evidence", () => {
  const query = buildRetrievalQuery("继续拆执行步骤", [
    {
      role: "assistant",
      content: "主图影响点击率。",
      answerEffectiveness: {
        status: "resolved",
        question: "主图视觉点击率转化率怎么优化？",
      },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            quote: "产品首图极大程度上决定了点击率。",
            title: "你是如何解决转化率的？",
          },
        ],
      },
    },
  ]);

  assert.match(query, /用户确认上轮回答有效/);
  assert.match(query, /主图视觉点击率转化率怎么优化/);
  assert.match(query, /上轮已引用原文证据/);
  assert.match(query, /产品首图极大程度上决定了点击率/);
  assert.doesNotMatch(query, /主图影响点击率/);
});

test("buildQaPayload uses conversation context for short follow-up questions", () => {
  const retrievalQuery = buildRetrievalQuery("那应该先改哪一块？", [
    { role: "user", content: "主图视觉点击率转化率怎么优化？" },
    {
      role: "assistant",
      content: "先检查主图差异化、图片体系、文案视觉和流量来源。",
      evidenceChain: {
        claims: [
          {
            type: "source_evidence",
            quote: "主图是唯一一个纯视觉元素。一个真正具备接客资格的图片体系，必须包含差异化的主图、场景化的副图、尺寸细节对比图和核心功能演示图。",
            title: "很多亚马逊上的产品，压根不具备接客资格。",
          },
        ],
      },
    },
  ]);
  const payload = buildQaPayload("那应该先改哪一块？", MAIN_IMAGE_CONTEXT, retrievalQuery);

  assert.match(payload.answer, /先看主图点击率/);
  assert.match(payload.answer, /差异化|点击率|图片体系/);
  assert.ok(payload.sources.some((source) => source.title.includes("转化率")));
});

test("buildRetrievalQuery keeps long histories compact", () => {
  const history = Array.from({ length: 20 }, (_, index) => ({
    role: index % 2 === 0 ? "user" : "assistant",
    content: `第${index}轮 ${"关键词".repeat(80)}`,
  }));
  const query = buildRetrievalQuery("继续说", history);

  assert.ok(query.length < 1800);
  assert.doesNotMatch(query, /第0轮/);
  assert.match(query, /第18轮/);
});

test("buildRetrievalQuery keeps older user-confirmed evidence as conversation memory", () => {
  const history = [
    { role: "user", content: "主图视觉点击率转化率怎么优化？" },
    {
      role: "assistant",
      content: "先检查主图和点击入口。",
      answerEffectiveness: { status: "resolved", question: "主图视觉点击率转化率怎么优化？" },
      evidenceFeedback: { "source-evidence:0": "useful" },
      evidenceChain: {
        claims: [
          {
            id: "source-evidence:0",
            type: "source_evidence",
            quote: "产品首图极大程度上决定了点击率。",
            title: "你是如何解决转化率的？",
          },
        ],
      },
    },
    ...Array.from({ length: 12 }, (_, index) => ({
      role: index % 2 === 0 ? "user" : "assistant",
      content: `后续闲聊 ${index}`,
    })),
  ];

  const query = buildRetrievalQuery("那第二步继续拆什么？", history);

  assert.match(query, /用户标记有用的原文/);
  assert.match(query, /产品首图极大程度上决定了点击率/);
  assert.match(query, /后续闲聊 10/);
  assert.ok(query.length < 1800);
});

test("buildQaPayload creates action-oriented sections for visual conversion questions", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);

  assert.match(payload.answer, /可执行结论/);
  assert.match(payload.answer, /执行顺序/);
  assert.match(payload.answer, /先看主图|先看点击率|主图/);
  assert.match(payload.answer, /作者视角/);
});

test("buildQaPayload keeps persona questions out of visual-conversion templates", () => {
  const question = "人群画像应该怎么构建？有哪些实操指导建议";
  const retrievalQuery = `${question}\n检索补充：用户画像 竞品信息 搜索词 基本功 词库 竞品库 产品文案 产品视觉 目标客户是谁`;
  const payload = buildQaPayload(question, PERSONA_CONTEXT, retrievalQuery);

  assert.ok(payload.sources.length >= 1);
  assert.match(payload.answer, /目标客户|用户画像|竞品信息|搜索词/);
  assert.match(payload.answer, /明确目标客户/);
  assert.doesNotMatch(payload.answer.slice(0, 420), /先把主图|主图点击率|主图差异化|视觉转化/);
});

test("buildQaPayload discloses stable template fallback cost separately from local model answers", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT, "主图视觉点击率转化率怎么优化？", {
    answerGeneration: {
      mode: "template_fallback",
      model: "stable-template",
      label: "稳定模板回答",
      summary: "本地模型未生成可校验回答，已回退到稳定模板。",
      boundary: "模板回答只整理本轮检索到的来源和摘录。",
    },
  });

  assert.equal(payload.answerGeneration.mode, "template_fallback");
  assert.equal(payload.usageFootprint.mode, "template_fallback");
  assert.equal(payload.usageFootprint.model, "stable-template");
  assert.match(payload.usageFootprint.summary, /本地稳定模板/);
  assert.match(payload.answerGeneration.boundary, /模板回答/);
});

test("buildQaPayload creates a source decision table without promoting action advice to evidence", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);
  const table = payload.sourceDecisionTable;

  assert.equal(table.title, "来源决策表");
  assert.ok(table.rows.length >= 1);
  assert.ok(table.rows.every((row) => Number.isInteger(row.sourceIndex)));
  assert.ok(table.rows.every((row) => row.quote && row.supports && row.cannotProve && row.validation));
  assert.ok(table.rows.every((row) => row.canUseAsEvidence === false && row.sourceCanUseAsEvidence === true));
  assert.match(table.boundary, /不是新的作者原文证据/);
});

test("buildQaPayload keeps source decision table conservative without author evidence", () => {
  const payload = buildQaPayload("量子芯片低温纠错架构怎么搭？", MAIN_IMAGE_CONTEXT);
  const table = payload.sourceDecisionTable;

  assert.equal(table.status, "needs_source");
  assert.deepEqual(table.rows, []);
  assert.match(table.summary, /没有可绑定的作者原文|不能生成/);
  assert.match(table.boundary, /没有原文/);
});

test("buildAnswerGraph creates question, concept, source, and author nodes", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);
  const graph = buildAnswerGraph(payload.question, payload.answer, payload.sources, payload.question, payload.rankedEvidence, payload.evidenceChain);

  assert.equal(graph.nodes[0].id, "question");
  assert.equal(graph.nodes[0].type, "question");
  assert.ok(graph.nodes.some((node) => node.id === "concept:主图" && node.type === "concept"));
  assert.ok(graph.nodes.some((node) => node.type === "source" && node.sourceIndex === 0));
  assert.ok(graph.nodes.some((node) => node.id === "author:跨境电商长期主义" && node.type === "author"));
  assert.match(graph.nodes[0].prompt, /继续拆成学习路线/);
  assert.ok(graph.nodes.some((node) => node.type === "concept" && /继续学习/.test(node.prompt || "")));
  assert.ok(graph.nodes.some((node) => node.type === "point" && /继续展开/.test(node.prompt || "")));
  assert.ok(graph.nodes.some((node) => node.type === "step" && /低风险检查清单/.test(node.prompt || "")));
  assert.ok(graph.edges.some((edge) => edge.from === "question" && edge.to === "concept:主图"));
  assert.ok(graph.edges.some((edge) => edge.type === "related_source" && edge.strength === "related"));
  assert.ok(graph.edges.some((edge) => edge.type === "supported_by" && edge.strength === "evidence"));
  assert.ok(graph.edges.some((edge) => edge.type === "written_by"));
});

test("buildAnswerGraph redacts product facts from node follow-up prompts", () => {
  const question = "我这个 garlic press，竞品 OXO，供应商报价 2.3 美金，CTR 0.25%，ACOS 45%，核心关键词 garlic press，ASIN B001234567，主图该先改什么？";
  const payload = buildQaPayload(question, MAIN_IMAGE_CONTEXT, question, {
    productInput: { text: question },
  });
  const questionNode = payload.graph.nodes.find((node) => node.id === "question");

  assert.ok(questionNode);
  assert.match(questionNode.prompt, /CTR/);
  assert.match(questionNode.prompt, /ACOS/);
  assert.match(questionNode.prompt, /ASIN/);
  assert.doesNotMatch(questionNode.prompt, /ASIN\s+ASIN/i);
  assert.doesNotMatch(questionNode.prompt, /0\.25|45%|garlic press|B001234567|OXO|2\.3/i);
});

test("buildQaPayload keeps same-title sources distinct when source paths differ", () => {
  const duplicateTitleContext = `Query: 主图点击率怎么判断？

跨境电商长期主义 2024-01-01 同名文章: # 同名文章
作者：跨境电商长期主义
发布时间：2024-01-01 08:00:00
原文链接：https://mp.weixin.qq.com/s/same-a
来源文件：跨境电商长期主义html/same-a.html
主图点击率判断要先看图片是否和竞品有差异。

跨境电商长期主义 2024-01-01 同名文章: # 同名文章
作者：跨境电商长期主义
发布时间：2024-01-01 09:00:00
原文链接：https://mp.weixin.qq.com/s/same-b
来源文件：跨境电商长期主义html/same-b.html
主图点击率第二篇独有证据是蓝色包装能形成货架差异。`;
  const payload = buildQaPayload("主图点击率蓝色包装怎么判断？", duplicateTitleContext);
  const sourcePaths = payload.sources.map((source) => source.sourcePath);
  const blueEvidence = payload.rankedEvidence.find((item) => /蓝色包装/.test(item.quote || ""));

  assert.ok(sourcePaths.includes("跨境电商长期主义html/same-a.html"));
  assert.ok(sourcePaths.includes("跨境电商长期主义html/same-b.html"));
  assert.ok(blueEvidence);
  assert.equal(payload.sources[blueEvidence.sourceIndex].sourcePath, "跨境电商长期主义html/same-b.html");
});

test("buildAnswerGraph links answer claims through evidence nodes to sources", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);
  const graph = payload.graph;
  const point = graph.nodes.find((node) => node.type === "point" && node.claimId === "system-inference:0");
  const sourceClaim = payload.evidenceChain.claims.find(
    (claim) => claim.type === "source_evidence" && /点击率|主图|转化率/.test(claim.quote || claim.text || ""),
  );
  const evidence = graph.nodes.find((node) => node.type === "evidence" && node.claimId === sourceClaim?.id);
  const source = graph.nodes.find((node) => node.type === "source" && node.sourceIndex === evidence?.sourceIndex);

  assert.ok(point);
  assert.ok(sourceClaim);
  assert.ok(evidence);
  assert.ok(source);
  assert.ok(payload.sources[evidence.sourceIndex].excerpt.includes(sourceClaim.quote));
  assert.ok(graph.edges.some((edge) => edge.from === point.id && edge.to === evidence.id && edge.type === "supported_by"));
  assert.ok(graph.edges.some((edge) => edge.from === evidence.id && edge.to === source.id && edge.type === "quoted_from"));
});

test("buildAnswerGraph keeps a lightweight graph when there are no sources", () => {
  const graph = buildAnswerGraph("没有资料的问题", "这次没有从本地知识库里找到足够相关的资料。", []);

  assert.ok(graph.nodes.some((node) => node.id === "question"));
  assert.ok(graph.nodes.some((node) => node.id === "empty:sources" && node.label === "暂无来源支撑"));
  assert.ok(graph.nodes.some((node) => node.id === "empty:sources" && /重新检索作者资料/.test(node.prompt || "")));
  assert.ok(graph.edges.some((edge) => edge.from === "question" && edge.to === "empty:sources"));
});

test("buildQaPayload includes graph data and follow-up context concepts", () => {
  const retrievalQuery = buildRetrievalQuery("那应该先改哪一块？", [
    { role: "user", content: "主图视觉点击率转化率怎么优化？" },
    {
      role: "assistant",
      content: "先检查主图差异化、图片体系、文案视觉和流量来源。",
    },
  ]);
  const payload = buildQaPayload("那应该先改哪一块？", MAIN_IMAGE_CONTEXT, retrievalQuery);

  assert.ok(Array.isArray(payload.graph.nodes));
  assert.ok(Array.isArray(payload.graph.edges));
  assert.ok(payload.graph.nodes.some((node) => node.id === "concept:主图"));
  assert.ok(payload.graph.nodes.some((node) => node.id === "concept:点击率"));
  assert.ok(payload.graph.nodes.some((node) => node.type === "source"));
});

test("buildLearningCard identifies intent and creates action follow-ups", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);
  const card = buildLearningCard(payload.question, payload.answer, payload.sources);

  assert.equal(card.intent.type, "visual");
  assert.match(card.intent.label, /视觉|转化/);
  assert.ok(card.nextActions.some((item) => item.includes("主图")));
  assert.ok(card.followUps.some((item) => item.includes("具体产品") || item.includes("页面")));
  assert.ok(card.evidence.some((item) => item.author === "跨境电商长期主义"));
  assert.ok(card.studyChecks.length >= 3);
  assert.ok(card.studyChecks.some((item) => item.kind === "source" && item.sourceIndex === 0));
  assert.ok(card.studyChecks.every((item) => item.boundary.includes("不产生新证据")));
});

test("buildQaPayload creates a per-answer learning queue from evidence, checks, validation, and dossier actions", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);

  assert.ok(payload.learningQueue);
  assert.equal(payload.learningQueue.progress.total, payload.learningQueue.items.length);
  assert.equal(payload.learningQueue.items[0].kind, "evidence");
  assert.equal(payload.learningQueue.items[0].action, "review-evidence");
  assert.equal(payload.learningQueue.items[0].completionMode, "evidence_feedback");
  assert.ok(payload.learningQueue.items.some((item) => item.kind === "study_check" && item.prompt && item.requiresEvidenceGate && item.locked));
  assert.ok(payload.learningQueue.items.some((item) => item.kind === "data" && item.action === "fill-data" && item.requiresEvidenceGate && item.locked));
  assert.ok(payload.learningQueue.items.some((item) => item.kind === "experiment" && item.action === "review-validation" && item.requiresEvidenceGate && item.locked));
  assert.ok(payload.learningQueue.items.some((item) => item.kind === "dossier" && item.action === "save-dossier" && item.requiresEvidenceGate && item.locked));
  assert.ok(payload.learningQueue.items.slice(1).every((item) => item.requiresEvidenceGate && item.locked));
  assert.match(payload.learningQueue.boundary, /不代表结论正确/);
});

test("buildLearningQueue starts with source search when no source supports the answer", () => {
  const payload = buildQaPayload("这个资料库里完全没有的主题应该怎么做？", "");

  assert.equal(payload.learningQueue.items[0].kind, "source_search");
  assert.equal(payload.learningQueue.items[0].action, "fill-prompt");
  assert.notEqual(payload.learningQueue.items[0].kind, "dossier");
  assert.ok(payload.learningQueue.items.every((item) => item.kind !== "evidence"));
  assert.match(payload.learningQueue.items[0].boundary, /不能沉淀成知识结论/);
});

test("buildLearningQueue still blocks later steps when sources exist without source evidence", () => {
  const queue = buildLearningQueue({
    sources: [{ author: "张子卿", title: "候选来源", excerpt: "只有泛泛内容。" }],
    evidenceChain: {
      claims: [{ id: "needs-source:0", type: "needs_source", text: "缺少可采纳原文证据" }],
    },
    validationPack: {
      dataRequests: [{ id: "keyword", label: "核心关键词", why: "用于下一轮判断" }],
      experiments: [{ id: "test", title: "低风险测试" }],
    },
    learningCard: {
      studyChecks: [{ id: "check", question: "来源支撑了什么？", expectedAnswer: "暂未支撑。", prompt: "请重新查来源。" }],
    },
    workflowIntent: {
      type: "method_learning",
      nextPrompt: "请换一种更具体的问法重新检索作者资料。",
    },
  });

  assert.equal(queue.items[0].kind, "source_search");
  assert.ok(queue.items.slice(1).every((item) => item.requiresEvidenceGate && item.locked));
  assert.ok(queue.items.some((item) => item.kind === "dossier" && item.lockedLabel === "先补来源"));
});

test("buildLearningQueue can be called directly with existing learning artifacts", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);
  const queue = buildLearningQueue({
    sources: payload.sources,
    evidenceChain: payload.evidenceChain,
    evidenceAudit: payload.evidenceAudit,
    validationPack: payload.validationPack,
    learningCard: payload.learningCard,
    workflowIntent: payload.workflowIntent,
  });

  assert.equal(queue.progress.completed, 0);
  assert.ok(queue.currentItemId);
  assert.ok(queue.items.length >= 4);
  assert.equal(queue.items[0].completionMode, "evidence_feedback");
  assert.ok(queue.items.slice(1).every((item) => item.requiresEvidenceGate && item.locked));
});

test("buildLearningQueue keeps evidence before conflicts and locks every later step", () => {
  const queue = buildLearningQueue({
    sources: [
      { author: "张子卿", title: "主图判断", excerpt: "先看点击率和转化率。" },
      { author: "飞翔的波波", title: "页面判断", excerpt: "页面判断要看整体。" },
    ],
    evidenceChain: {
      claims: [
        {
          id: "claim:0",
          type: "source_evidence",
          sourceIndex: 0,
          quote: "先看点击率和转化率。",
        },
      ],
    },
    evidenceAudit: {
      conflictSignals: [{ concept: "主图", sourceIndexes: [0, 1] }],
    },
    validationPack: {
      dataRequests: [{ id: "ctr", why: "需要真实点击率" }],
      experiments: [{ id: "main-image", title: "主图 A/B 验证" }],
    },
    learningCard: {
      studyChecks: [{ id: "source", question: "原文支撑了什么？", prompt: "请解释来源。" }],
    },
    workflowIntent: {
      nextPrompt: "继续拆下一步。",
      primaryAction: "先核对关键来源。",
    },
  });

  assert.equal(queue.items[0].id, "queue:evidence");
  assert.equal(queue.items[1].id, "queue:conflict");
  assert.ok(queue.items.slice(1).every((item) => item.requiresEvidenceGate && item.locked));
  assert.ok(queue.items.some((item) => item.kind === "study_check"));
  assert.ok(queue.items.some((item) => item.kind === "data"));
  assert.ok(queue.items.some((item) => item.kind === "experiment"));
});

test("buildQaPayload identifies method learning workflow intent", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);

  assert.equal(payload.workflowIntent.type, "method_learning");
  assert.match(payload.workflowIntent.label, /方法学习/);
  assert.match(payload.workflowIntent.primaryAction, /核对关键来源/);
  assert.match(payload.workflowIntent.boundary, /作者原文/);
});

test("buildQaPayload identifies product diagnosis workflow intent", () => {
  const question = [
    "请结合这些产品信息判断下一步：",
    "主图是白底图，CTR 0.25%，CVR 5%",
    "SP 广告 ACOS 45%，核心关键词 garlic press",
  ].join("\n");
  const payload = buildQaPayload(question, MAIN_IMAGE_CONTEXT, question, {
    productInput: { text: question },
  });

  assert.equal(payload.workflowIntent.type, "product_diagnosis");
  assert.match(payload.workflowIntent.label, /产品诊断/);
  assert.match(payload.workflowIntent.primaryAction, /诊断优先级/);
  assert.match(payload.workflowIntent.boundary, /不是作者原文证据/);
});

test("buildQaPayload gives product diagnosis priority over ambiguous rejudgement wording", () => {
  const question = "帮我重新判断这个产品值不值得做，核心关键词 garlic press，CTR 0.25%，ACOS 45%";
  const payload = buildQaPayload(question, MAIN_IMAGE_CONTEXT, question, {
    productInput: { text: question },
  });

  assert.equal(payload.workflowIntent.type, "product_diagnosis");
  assert.match(payload.workflowIntent.boundary, /用户产品材料不是作者原文证据/);
});

test("buildQaPayload identifies experiment review workflow intent", () => {
  const question = [
    "我补充了验证数据，请判断下一步：",
    "实验名称：主图 A/B",
    "CTR 前/后：0.25% / 0.42%",
    "CVR 前/后：5% / 5.2%",
    "结论：点击改善，转化还没动",
  ].join("\n");
  const payload = buildQaPayload(question, MAIN_IMAGE_CONTEXT, question, {
    productInput: { text: question },
  });

  assert.equal(payload.workflowIntent.type, "experiment_review");
  assert.match(payload.workflowIntent.label, /实验复盘/);
  assert.match(payload.workflowIntent.primaryAction, /回看实验前后数据/);
  assert.match(payload.workflowIntent.boundary, /不改写作者原文/);
});

test("buildQaPayload does not treat method review wording as experiment review", () => {
  const payload = buildQaPayload("广告复盘的方法是什么？前后顺序怎么排？", MAIN_IMAGE_CONTEXT);

  assert.equal(payload.workflowIntent.type, "method_learning");
  assert.match(payload.workflowIntent.boundary, /作者原文/);
});

test("buildQaPayload routes no-source answers to source search intent", () => {
  const payload = buildQaPayload("这个资料库里完全没有的主题应该怎么做？", "");

  assert.equal(payload.workflowIntent.type, "source_search");
  assert.match(payload.workflowIntent.label, /补来源检索/);
  assert.match(payload.workflowIntent.boundary, /不能沉淀成知识结论/);
});

test("buildQaPayload treats product-specific short questions as diagnosis even without full metrics", () => {
  const payload = buildQaPayload("我这个 Listing 该先改哪？", MAIN_IMAGE_CONTEXT);

  assert.equal(payload.workflowIntent.type, "product_diagnosis");
  assert.equal(payload.workflowIntent.confidence, "medium");
});

test("buildQaPayload honors a user-confirmed workflow intent preference", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT, undefined, {
    intentPreference: { type: "product_diagnosis" },
  });

  assert.equal(payload.workflowIntent.type, "product_diagnosis");
  assert.equal(payload.workflowIntent.confidence, "user_confirmed");
  assert.match(payload.workflowIntent.boundary, /用户产品材料不是作者原文证据/);
});

test("buildWorkflowIntent can expose selectable intent templates", () => {
  const intent = buildWorkflowIntent({
    question: "主图视觉点击率转化率怎么优化？",
    sources: [{ title: "你是如何解决转化率的？" }],
    intentPreference: "experiment_review",
  });

  assert.equal(intent.type, "experiment_review");
  assert.equal(intent.confidence, "user_confirmed");
  assert.match(intent.primaryAction, /实验前后数据/);
});

test("buildLearningCard keeps a useful study card without sources", () => {
  const card = buildLearningCard("完全没有命中的问题", "这次没有从本地知识库里找到足够相关的资料。", []);

  assert.equal(card.evidence.length, 0);
  assert.deepEqual(card.studyChecks, []);
  assert.ok(card.missingInputs.some((item) => item.includes("具体")));
  assert.ok(card.followUps.length >= 2);
});

test("buildQaPayload includes a learning card with follow-up context", () => {
  const retrievalQuery = buildRetrievalQuery("那应该先改哪一块？", [
    { role: "user", content: "主图视觉点击率转化率怎么优化？" },
    { role: "assistant", content: "先检查主图差异化、图片体系、文案视觉和流量来源。" },
  ]);
  const payload = buildQaPayload("那应该先改哪一块？", MAIN_IMAGE_CONTEXT, retrievalQuery);

  assert.equal(payload.learningCard.intent.type, "visual");
  assert.ok(payload.learningCard.nextActions.some((item) => item.includes("主图")));
  assert.ok(payload.learningCard.followUps.some((item) => item.includes("Listing") || item.includes("产品")));
});

test("buildQaPayload creates a source study pack with explicit evidence identity", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);

  assert.ok(payload.sourceStudyPack);
  assert.equal(payload.sourceStudyPack.status, "needs_review");
  assert.match(payload.sourceStudyPack.boundary, /确认有用的作者原文证据/);
  assert.ok(payload.sourceStudyPack.sourceCards.length >= 1);
  assert.ok(payload.sourceStudyPack.flashcards.length >= 1);
  assert.ok(payload.sourceStudyPack.concepts.some((item) => /主图|点击率|转化率/.test(item.label)));
  for (const card of payload.sourceStudyPack.sourceCards) {
    assert.equal(card.identity, "作者原文");
    assert.equal(card.canUseAsEvidence, true);
    assert.ok(card.claimId);
    assert.ok(Number.isInteger(card.sourceIndex));
    assert.ok(card.title);
    assert.ok(card.quote);
  }
  for (const card of payload.sourceStudyPack.flashcards) {
    assert.equal(card.identity, "系统整理");
    assert.equal(card.canUseAsEvidence, false);
    assert.ok(card.claimId);
    assert.ok(Number.isInteger(card.sourceIndex));
    assert.match(card.boundary, /不是新的作者证据/);
  }
});

test("buildQaPayload creates a per-answer topic source tree from question, concepts, authors, and sources", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);

  assert.ok(payload.topicSourceTree);
  assert.equal(payload.topicSourceTree.title, "本轮主题来源树");
  assert.equal(payload.topicSourceTree.status, "ready");
  assert.match(payload.topicSourceTree.boundary, /只基于本轮可引用的作者原文/);
  assert.ok(payload.topicSourceTree.topic.label.includes("主图"));
  assert.ok(payload.topicSourceTree.concepts.some((item) => /主图|点击率|转化率|Listing|广告/.test(item.label)));
  assert.ok(payload.topicSourceTree.sources.length >= 1);
  assert.ok(payload.topicSourceTree.authors.some((item) => item.author === "跨境电商长期主义"));
  assert.ok(payload.topicSourceTree.paths.some((item) => item.kind === "author"));
  assert.ok(payload.topicSourceTree.nextPrompts.some((prompt) => /继续追问|先改|核对|来源/.test(prompt)));
  for (const source of payload.topicSourceTree.sources) {
    assert.equal(source.identity, "作者原文");
    assert.equal(source.canUseAsEvidence, true);
    assert.ok(Number.isInteger(source.sourceIndex));
    assert.ok(payload.sources[source.sourceIndex]);
  }
});

test("buildTopicSourceTree stays conservative without sources", () => {
  const tree = buildTopicSourceTree({
    question: "完全没有命中的问题",
    answer: "我没有找到来源。",
    sources: [],
    retrievalQuestion: "完全没有命中的问题",
  });

  assert.equal(tree.status, "needs_source");
  assert.equal(tree.sources.length, 0);
  assert.equal(tree.authors.length, 0);
  assert.ok(tree.concepts.length >= 1);
  assert.match(tree.boundary, /没有可绑定的作者原文/);
  assert.ok(tree.paths.some((item) => item.kind === "gap"));
});

test("buildQaPayload topic source tree inherits concepts for short follow-up questions", () => {
  const retrievalQuery = "主图视觉点击率转化率怎么优化？\n那我应该先改哪一块？";
  const payload = buildQaPayload("那我应该先改哪一块？", MAIN_IMAGE_CONTEXT, retrievalQuery);
  const labels = payload.topicSourceTree.concepts.map((item) => item.label).join(" ");

  assert.match(labels, /主图|点击率|转化率/);
  assert.ok(payload.topicSourceTree.sources.length >= 1);
  assert.ok(payload.topicSourceTree.nextPrompts.some((prompt) => /上一问|主图|先改/.test(prompt)));
});

test("buildQaPayload topic source tree does not turn workflow memory into author sources", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", WORKFLOW_MEMORY_RESULT);

  assert.equal(payload.topicSourceTree.status, "needs_source");
  assert.equal(payload.topicSourceTree.sources.length, 0);
  assert.equal(payload.topicSourceTree.authors.length, 0);
  assert.match(payload.topicSourceTree.boundary, /作者原文/);
});

test("buildQaPayload keeps source study pack conservative without sources", () => {
  const payload = buildQaPayload("完全没有命中的问题", "");

  assert.ok(payload.sourceStudyPack);
  assert.equal(payload.sourceStudyPack.status, "needs_source");
  assert.equal(payload.sourceStudyPack.sourceCards.length, 0);
  assert.equal(payload.sourceStudyPack.flashcards.length, 0);
  assert.ok(payload.sourceStudyPack.gaps.some((item) => /来源|原文/.test(item.label + item.reason)));
  assert.match(payload.sourceStudyPack.boundary, /不能生成正式研读包/);
});

test("buildSourceContextFromArticle locates quote with neighboring original paragraphs", () => {
  const source = {
    author: "跨境电商长期主义",
    date: "2022-06-18",
    title: "你是如何解决转化率的？",
    sourceUrl: "https://mp.weixin.qq.com/s/example6",
    sourcePath: "跨境电商长期主义html/example.html",
    excerpt: "产品首图极大程度上决定了点击率，但是如果没有点击率，转化率就成了一个没有必要的数据指标。",
  };
  const context = buildSourceContextFromArticle(source, `# 你是如何解决转化率的？
作者：跨境电商长期主义
发布时间：2022-06-18 08:50:45
原文链接：https://mp.weixin.qq.com/s/example6
来源文件：跨境电商长期主义html/example.html
前一段说明流量来源和转化率之间有关系。

产品首图极大程度上决定了点击率，但是如果没有点击率，转化率就成了一个没有必要的数据指标。

后一段继续讲页面文案、视觉和对比图的承接。`, {
    quote: "产品首图极大程度上决定了点击率，但是如果没有点击率，转化率就成了一个没有必要的数据指标。",
  });

  assert.equal(context.status, "located");
  assert.equal(context.identity, "作者原文");
  assert.match(context.before, /前一段说明/);
  assert.match(context.match, /产品首图极大程度上决定了点击率/);
  assert.match(context.after, /后一段继续讲/);
  assert.equal(context.canUseAsEvidence, true);
});

test("buildSourceContextFromArticle refuses to invent context when quote is not located", () => {
  const context = buildSourceContextFromArticle({
    author: "飞翔的波波",
    date: "2026-01-16",
    title: "知道了竞品的转化率，才能真正做好亚马逊",
    sourcePath: "飞翔的波波html/example.html",
    excerpt: "这段引用在文章里并不存在。",
  }, `# 知道了竞品的转化率，才能真正做好亚马逊
作者：飞翔的波波
发布时间：2026-01-16 12:01:34
原文链接：https://mp.weixin.qq.com/s/example7
来源文件：飞翔的波波html/example.html
真实原文只讨论竞品转化率和关键词，不包含测试引用。`, {
    quote: "完全不存在的测试引用",
  });

  assert.equal(context.status, "not_located");
  assert.equal(context.identity, "作者原文");
  assert.match(context.reason, /未定位到原文上下文/);
  assert.equal(context.canUseAsEvidence, false);
});

test("buildAuthorPerspectiveRoom groups source evidence by author without promoting it", () => {
  const compareQuestion = "三位作者怎么看主图视觉点击率转化率优化？";
  const payload = buildQaPayload(compareQuestion, `Query: ${compareQuestion}

飞翔的波波 2026-04-01 主图点击率优先级: # 主图点击率优先级
作者：飞翔的波波
发布时间：2026-04-01 10:00:00
原文链接：https://mp.weixin.qq.com/s/author-a
来源文件：飞翔的波波html/author-a.html
新品前期必须先优化主图点击率，主图不突出，广告再多也很难把点击拉起来。

张子卿 2026-04-02 主图不是唯一入口: # 主图不是唯一入口
作者：张子卿
发布时间：2026-04-02 10:00:00
原文链接：https://mp.weixin.qq.com/s/author-b
来源文件：张子卿html/author-b.html
不建议一上来只改主图，要先看价格、评价和页面承接，否则点击率提高也可能不转化。

跨境电商长期主义 2026-04-03 视觉体系要长期复用: # 视觉体系要长期复用
作者：跨境电商长期主义
发布时间：2026-04-03 10:00:00
原文链接：https://mp.weixin.qq.com/s/author-c
来源文件：跨境电商长期主义html/author-c.html
产品首图决定点击入口，但视觉体系还要承接副图、对比图和 Listing 页面，不能只做一张图。`);
  const room = buildAuthorPerspectiveRoom({
    question: payload.question,
    sources: payload.sources,
    evidenceChain: payload.evidenceChain,
    retrievalQuestion: payload.question,
  });

  assert.equal(room.status, "ready");
  assert.match(room.boundary, /待核对/);
  assert.equal(room.authors.length, 3);
  assert.ok(room.sharedConcepts.some((item) => item.label === "主图"));
  assert.ok(room.differences.length >= 1);
  for (const author of room.authors) {
    assert.ok(author.author);
    assert.ok(author.items.length >= 1);
    for (const item of author.items) {
      assert.equal(item.identity, "作者原文");
      assert.equal(item.canUseAsEvidence, false);
      assert.ok(Number.isInteger(item.sourceIndex));
      assert.ok(item.claimId);
      assert.ok(item.quote);
    }
  }
});

test("buildQaPayload diversifies authors for explicit author comparison questions", () => {
  const compareQuestion = "请对比张子卿、飞翔的波波、跨境电商长期主义三位作者关于主图点击率和转化率优化的观点。";
  const dominantLongTermEvidence = Array.from({ length: 10 }, (_, index) =>
    `主图点击率转化率优化第 ${index + 1} 条：产品首图决定点击入口，但还要看副图、Listing 页面和广告流量承接。`,
  ).join("\n");
  const payload = buildQaPayload(compareQuestion, `Query: ${compareQuestion}

跨境电商长期主义 2026-04-03 视觉体系要长期复用: # 视觉体系要长期复用
作者：跨境电商长期主义
发布时间：2026-04-03 10:00:00
原文链接：https://mp.weixin.qq.com/s/long-term
来源文件：跨境电商长期主义html/long-term.html
${dominantLongTermEvidence}

张子卿 2026-04-02 主图不是唯一入口: # 主图不是唯一入口
作者：张子卿
发布时间：2026-04-02 10:00:00
原文链接：https://mp.weixin.qq.com/s/zhang
来源文件：张子卿html/zhang.html
不建议一上来只改主图，要先看价格、评价和页面承接，否则点击率提高也可能不转化。

飞翔的波波 2026-04-01 主图点击率优先级: # 主图点击率优先级
作者：飞翔的波波
发布时间：2026-04-01 10:00:00
原文链接：https://mp.weixin.qq.com/s/bobo
来源文件：飞翔的波波html/bobo.html
新品前期必须先优化主图点击率，主图不突出，广告再多也很难把点击拉起来。`);

  const sourceAuthors = new Set(payload.sources.map((source) => source.author));
  const roomAuthors = new Set(payload.authorPerspectiveRoom.authors.map((author) => author.author));

  assert.equal(payload.authorPerspectiveRoom.status, "ready");
  assert.ok(sourceAuthors.has("张子卿"));
  assert.ok(sourceAuthors.has("飞翔的波波"));
  assert.ok(sourceAuthors.has("跨境电商长期主义"));
  assert.ok(roomAuthors.has("张子卿"));
  assert.ok(roomAuthors.has("飞翔的波波"));
  assert.ok(roomAuthors.has("跨境电商长期主义"));
});

test("buildQaPayload turns conflict data into a bounded business priority decision", () => {
  const question = "请对比三位作者关于主图点击率和转化率优化的观点，并结合我的数据判断先改哪里。";
  const payload = buildQaPayload(question, `Query: ${question}

飞翔的波波 2026-04-01 亚马逊运营真相：影响点击率的，从来不是主图: # 亚马逊运营真相：影响点击率的，从来不是主图
作者：飞翔的波波
发布时间：2026-04-01 10:00:00
原文链接：https://mp.weixin.qq.com/s/bobo-ctr
来源文件：飞翔的波波html/bobo-ctr.html
影响点击率的从来不是只看主图，还要看价格、评价和搜索结果里的整体吸引力。

跨境电商长期主义 2022-06-18 你是如何解决转化率的？: # 你是如何解决转化率的？
作者：跨境电商长期主义
发布时间：2022-06-18 08:50:45
原文链接：https://mp.weixin.qq.com/s/long-cvr
来源文件：跨境电商长期主义html/long-cvr.html
产品首图极大程度上决定了点击率，但是进入商品页面之后的提升，一方面靠文案和视觉，还有很重要的一点就是对比。`, question, {
    productInput: {
      text: [
        "主图：白底图，和前三个竞品很像，没有明显差异。",
        "当前点击率 CTR：0.22%，曝光 12000，点击 26。",
        "当前转化率 CVR：8.3%，Session 313，订单 26。",
        "核心关键词下前三竞品：价格接近，评价数量接近。",
      ].join("\n"),
    },
  });

  const decision = payload.validationPack.businessDecision;

  assert.equal(decision.status, "ready");
  assert.equal(decision.priority, "main_image");
  assert.match(decision.label, /优先改主图|点击入口/);
  assert.ok(decision.supportingData.some((item) => /CTR|点击率/.test(item.label + item.value)));
  assert.ok(decision.supportingData.some((item) => /CVR|转化率/.test(item.label + item.value)));
  assert.ok(!decision.missingData.some((item) => /CTR|点击率|CVR|转化率/.test(item.label)));
  assert.match(decision.boundary, /用户产品数据.*不是作者原文证据/);
  assert.match(payload.answer, /当前产品判断/);
});

test("buildQaPayload refuses a business priority decision when key data is missing", () => {
  const payload = buildQaPayload("主图和转化率应该先改哪一块？", MAIN_IMAGE_CONTEXT, undefined, {
    productInput: {
      text: [
        "主图：和竞品很像。",
        "当前点击率 CTR：0.31%，曝光 8000，点击 25。",
      ].join("\n"),
    },
  });

  const decision = payload.validationPack.businessDecision;

  assert.equal(decision.status, "insufficient_data");
  assert.equal(decision.priority, "insufficient");
  assert.match(decision.label, /数据不足|不能判断/);
  assert.ok(decision.missingData.some((item) => /CVR|转化率/.test(item.label)));
  assert.match(decision.boundary, /不要直接下最终判断|不能替代作者原文证据|不是作者原文证据/);
});

test("buildAuthorPerspectiveRoom stays conservative without source evidence", () => {
  const room = buildAuthorPerspectiveRoom({
    question: "完全没有命中的问题",
    sources: [],
    evidenceChain: { claims: [] },
  });

  assert.equal(room.status, "needs_source");
  assert.equal(room.authors.length, 0);
  assert.match(room.boundary, /不能生成跨作者观点/);
});

test("buildEvidenceChain separates source evidence from system inference", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);
  const chain = buildEvidenceChain(payload.question, payload.answer, payload.sources, payload.rankedEvidence);

  assert.ok(chain.claims.some((claim) => claim.type === "source_evidence"));
  assert.ok(chain.claims.some((claim) => claim.type === "system_inference"));
  const sourceClaim = chain.claims.find((claim) => claim.type === "source_evidence");
  assert.equal(sourceClaim.sourceIndex, 0);
  assert.equal(sourceClaim.trustKind, "author_original");
  assert.equal(sourceClaim.trustLabel, "作者原文证据");
  assert.equal(sourceClaim.canUseAsEvidence, true);
  assert.match(sourceClaim.quote, /点击率|转化率|主图/);
  const inferenceClaim = chain.claims.find((claim) => claim.type === "system_inference");
  assert.equal(inferenceClaim.sourceIndex, undefined);
  assert.equal(inferenceClaim.trustKind, "system_synthesis");
  assert.equal(inferenceClaim.trustLabel, "二次摘要/系统整理");
  assert.equal(inferenceClaim.canUseAsEvidence, false);
  assert.match(inferenceClaim.validation, /验证|数据|页面/);
});

test("buildQaPayload exposes a source trust chain without promoting product input or synthesis", () => {
  const payload = buildQaPayload(
    "我这个 garlic press 主图和竞品差不多，CTR 0.25%，CVR 5%，竞品 ASIN B001234567，应该先改哪里？",
    MAIN_IMAGE_CONTEXT,
    "主图视觉点击率转化率怎么优化？",
    {
      productInput: {
        text: "我这个 garlic press 主图和竞品差不多，CTR 0.25%，CVR 5%，竞品 ASIN B001234567，应该先改哪里？",
      },
    },
  );

  assert.ok(payload.sourceTrust);
  assert.equal(payload.sourceTrust.title, "本轮来源核对状态");
  assert.equal(payload.sourceTrust.status, "source_backed");
  assert.match(payload.sourceTrust.boundary, /有来源不等于已经人工核验或业务验证完成/);
  assert.match(payload.sourceTrust.boundary, /用户产品材料不是作者原文证据/);
  assert.match(payload.sourceTrust.boundary, /实验复盘不是作者原文证据/);

  const byId = new Map(payload.sourceTrust.categories.map((item) => [item.id, item]));
  assert.equal(byId.get("author_original").label, "作者原文证据");
  assert.ok(byId.get("author_original").count >= 1);
  assert.ok(byId.get("author_original").claimIds.every((id) => payload.evidenceChain.claims.some((claim) => claim.id === id && claim.trustKind === "author_original")));
  assert.equal(byId.get("system_synthesis").label, "二次摘要/系统整理");
  assert.ok(byId.get("system_synthesis").count >= 1);
  assert.equal(byId.get("product_material").label, "用户产品材料");
  assert.ok(byId.get("product_material").count >= 1);
  assert.equal(byId.get("experiment_review").label, "实验/复盘");
  assert.match(byId.get("experiment_review").description, /不是作者原文证据/);
  assert.equal(byId.get("insufficient").label, "不足以确认");

  assert.ok(
    payload.evidenceChain.claims
      .filter((claim) => claim.trustLabel === "作者原文证据")
      .every((claim) => claim.type === "source_evidence" && claim.trustKind === "author_original"),
  );
  assert.doesNotMatch(JSON.stringify(payload.sourceTrust.categories), /B001234567|CTR 0\.25|CVR 5|garlic press/i);
});

test("buildQaPayload exposes source-tree calibration as routing, not author evidence", () => {
  const sourceTreeCalibration = {
    title: "OpenHuman 来源树辅助检索",
    status: "active",
    candidateCount: 2,
    resolvedSourceCount: 1,
    summaryHintCount: 1,
    summary: "OpenHuman 来源树辅助命中 2 个候选来源，其中 1 个已回到作者原文库核对。",
    boundary: "来源树摘要和候选片段只负责帮系统找路，不能当作者原文证据；回答里的引用必须回到本地作者原文上下文后才可采纳。",
    candidates: [
      {
        id: "source-tree:candidate:0",
        type: "route_hint",
        label: "你是如何解决转化率的？",
        matchedOriginalSource: true,
        canUseAsEvidence: false,
      },
    ],
    summaries: [
      {
        id: "summary-1",
        type: "summary_hint",
        label: "来源树摘要",
        canUseAsEvidence: false,
      },
    ],
  };
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT, undefined, {
    sourceTreeCalibration,
  });

  assert.equal(payload.sourceTreeCalibration.status, "active");
  assert.equal(payload.sourceTreeCalibration.candidateCount, 2);
  assert.equal(payload.sourceTreeCalibration.resolvedSourceCount, 1);
  assert.equal(payload.sourceTreeCalibration.candidates[0].canUseAsEvidence, false);
  assert.equal(payload.sourceTreeCalibration.summaries[0].canUseAsEvidence, false);
  assert.equal(payload.sourceTrust.sourceTree.status, "active");
  assert.equal(payload.sourceTrust.sourceTree.resolvedSourceCount, 1);
  assert.match(payload.sourceTrust.sourceTree.boundary, /不能当作者原文证据/);
  assert.ok(payload.evidenceChain.claims.every((claim) => claim.trustKind !== "source_tree_summary"));
  assert.doesNotMatch(JSON.stringify(payload.sourceTrust.categories), /来源树摘要/);
});

test("buildQaPayload does not promote candidate selected sources into author evidence", () => {
  const candidateContext = `Query: 主图视觉点击率转化率怎么优化？

跨境电商长期主义 2022-06-18 伪造来源: # 伪造来源
作者：跨境电商长期主义
发布时间：2022-06-18
来源状态：候选/待确认，必须先核对，不能直接当成已采纳证据。
主图点击率和转化率都已经被这个假来源确认了。`;

  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", candidateContext);

  assert.equal(payload.sources.length, 0);
  assert.equal(payload.sourceTrust.status, "needs_source");
  assert.ok(payload.evidenceChain.claims.every((claim) => claim.type !== "source_evidence"));
});

test("buildQaPayload annotates answer lines with inline evidence markers", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);

  assert.match(payload.answer, /先把主图当成点击率入口处理.*【推断1】/);
  assert.match(payload.answer, /先看主图点击率.*【行动1】/);
  assert.match(payload.answer, /产品首图极大程度上决定了点击率.*【证据\d+】/);
  assert.ok(payload.evidenceChain.claims.some((claim) => claim.id === "source-evidence:0"));
  const inferenceClaim = payload.evidenceChain.claims.find((claim) => claim.type === "system_inference");
  assert.doesNotMatch(inferenceClaim.text, /【推断\d+】/);
});

test("buildEvidenceChain classifies action advice as not directly proven by sources", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);
  const actionClaim = payload.evidenceChain.claims.find((claim) => claim.type === "action_advice");

  assert.ok(actionClaim);
  assert.equal(actionClaim.sourceIndex, undefined);
  assert.match(actionClaim.text, /主图|副图|广告/);
  assert.equal(actionClaim.canProve, false);
  assert.match(actionClaim.basis, /执行步骤|不是原文直接结论/);
});

test("buildQaPayload includes evidence chain and ranked evidence", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);

  assert.ok(Array.isArray(payload.rankedEvidence));
  assert.ok(payload.rankedEvidence.length >= 2);
  assert.ok(payload.rankedEvidence.every((item) => Number.isInteger(item.sourceIndex)));
  assert.ok(Array.isArray(payload.evidenceChain.claims));
  assert.ok(payload.evidenceChain.claims.some((claim) => claim.type === "source_evidence"));
  assert.ok(payload.evidenceChain.claims.some((claim) => claim.type === "system_inference"));
  assert.ok(payload.evidenceChain.claims.some((claim) => claim.type === "action_advice"));
});

test("buildQaPayload includes a source-bound synthesis answer", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);
  const sourceClaimIds = new Set(
    payload.evidenceChain.claims
      .filter((claim) => claim.type === "source_evidence")
      .map((claim) => claim.id),
  );

  assert.ok(payload.synthesisAnswer);
  assert.equal(payload.synthesisAnswer.status, "source_backed");
  assert.match(payload.synthesisAnswer.summary, /主图|点击率|转化率/);
  assert.ok(payload.synthesisAnswer.sourceCoverage.sourceCount >= 1);
  assert.ok(payload.synthesisAnswer.sourceCoverage.evidenceCount >= 1);
  assert.ok(payload.synthesisAnswer.points.length >= 1);
  for (const point of payload.synthesisAnswer.points) {
    assert.equal(point.identity, "系统综合");
    assert.equal(point.canUseAsEvidence, false);
    assert.ok(point.claimIds.length >= 1);
    assert.ok(point.claimIds.every((claimId) => sourceClaimIds.has(claimId)));
    assert.ok(point.support.every((item) => sourceClaimIds.has(item.claimId) && Number.isInteger(item.sourceIndex)));
  }
  assert.match(payload.synthesisAnswer.boundary, /系统综合/);
  assert.match(payload.synthesisAnswer.boundary, /作者原文/);
});

test("buildQaPayload synthesis stays conservative when no source evidence exists", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", "");

  assert.ok(payload.synthesisAnswer);
  assert.equal(payload.synthesisAnswer.status, "needs_source");
  assert.equal(payload.synthesisAnswer.points.length, 0);
  assert.equal(payload.synthesisAnswer.sourceClaimIds.length, 0);
  assert.ok(payload.synthesisAnswer.gaps.some((item) => /来源|原文/.test(`${item.label}${item.reason}`)));
  assert.match(payload.synthesisAnswer.boundary, /不能生成正式综合结论/);
});

test("buildQaPayload creates a source-bound notebook guide without promoting it to evidence", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);
  const sourceClaimIds = new Set(payload.evidenceChain.claims.filter((claim) => claim.type === "source_evidence").map((claim) => claim.id));

  assert.ok(payload.notebookGuide);
  assert.equal(payload.notebookGuide.title, "本轮学习简报");
  assert.equal(payload.notebookGuide.status, "source_backed");
  assert.ok(payload.notebookGuide.briefing.length >= 1);
  assert.ok(payload.notebookGuide.faq.length >= 1);
  assert.ok(payload.notebookGuide.quiz.length >= 1);
  assert.ok(payload.notebookGuide.glossary.length >= 1);
  assert.match(payload.notebookGuide.boundary, /系统整理/);
  assert.match(payload.notebookGuide.boundary, /不是作者原文证据/);

  for (const item of [
    ...payload.notebookGuide.briefing,
    ...payload.notebookGuide.faq,
    ...payload.notebookGuide.quiz,
    ...payload.notebookGuide.glossary,
    ...payload.notebookGuide.gaps,
  ]) {
    assert.equal(item.identity, "系统整理");
    assert.equal(item.canUseAsEvidence, false);
    assert.ok(item.claimIds.every((claimId) => sourceClaimIds.has(claimId)));
    assert.ok(item.sourceIndexes.every((sourceIndex) => Number.isInteger(sourceIndex) && payload.sources[sourceIndex]));
  }
});

test("buildQaPayload keeps notebook guide conservative without source evidence", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", "");

  assert.ok(payload.notebookGuide);
  assert.equal(payload.notebookGuide.status, "needs_source");
  assert.equal(payload.notebookGuide.briefing.length, 0);
  assert.equal(payload.notebookGuide.faq.length, 0);
  assert.equal(payload.notebookGuide.quiz.length, 0);
  assert.ok(payload.notebookGuide.gaps.some((item) => /来源|原文/.test(`${item.label}${item.reason}`)));
  assert.ok(payload.notebookGuide.gaps.every((item) => item.identity === "系统整理" && item.canUseAsEvidence === false));
  assert.match(payload.notebookGuide.boundary, /不能生成正式学习简报/);
});

test("buildQaPayload synthesis does not promote product input into source support", () => {
  const payload = buildQaPayload(
    "我这个 garlic press 主图和竞品差不多，CTR 0.25%，CVR 5%，竞品 ASIN B001234567，应该先改哪里？",
    MAIN_IMAGE_CONTEXT,
    "主图视觉点击率转化率怎么优化？",
    {
      productInput: {
        text: "我这个 garlic press 主图和竞品差不多，CTR 0.25%，CVR 5%，竞品 ASIN B001234567，应该先改哪里？",
      },
    },
  );

  assert.equal(payload.synthesisAnswer.status, "source_backed");
  assert.doesNotMatch(JSON.stringify(payload.synthesisAnswer), /B001234567|CTR 0\.25|CVR 5|garlic press/i);
  assert.ok(
    payload.synthesisAnswer.points.every((point) =>
      point.support.every((item) => item.identity === "作者原文" && item.evidenceKind === "source_evidence"),
    ),
  );
});

test("buildQaPayload synthesis does not bind unsupported question concepts to unrelated evidence", () => {
  const visualOnlyContext = `Query: 主图和广告应该怎么排优先级？

跨境电商长期主义 2026-04-01 主图点击率优先: # 主图点击率优先
作者：跨境电商长期主义
发布时间：2026-04-01 10:00:00
原文链接：https://mp.weixin.qq.com/s/visual-only
来源文件：跨境电商长期主义html/visual-only.html
产品首图极大程度上决定了点击率。搜索结果里图片不突出，点击率就很难起来。`;
  const payload = buildQaPayload("主图和广告应该怎么排优先级？", visualOnlyContext);

  assert.equal(payload.synthesisAnswer.status, "source_backed");
  assert.ok(payload.synthesisAnswer.points.some((point) => /主图|点击率/.test(`${point.label}${point.text}`)));
  assert.ok(!payload.synthesisAnswer.points.some((point) => /广告/.test(`${point.label}${point.text}`)));
  assert.ok(payload.synthesisAnswer.gaps.some((gap) => /广告/.test(`${gap.label}${gap.reason}`)));
});

test("buildQaPayload keeps source evidence quotes inside returned source excerpts", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);
  const sourceClaims = payload.evidenceChain.claims.filter((claim) => claim.type === "source_evidence");

  assert.ok(sourceClaims.length > 0);
  for (const claim of sourceClaims) {
    const source = payload.sources[claim.sourceIndex];
    assert.ok(source, `missing source for ${claim.id}`);
    assert.ok(!String(claim.quote || "").endsWith("..."));
    assert.ok(
      normalizeEvidenceTextForTest(source.excerpt).includes(normalizeEvidenceTextForTest(claim.quote)),
      `source excerpt should contain quote for ${claim.id}`,
    );
  }
});

test("buildQaPayload includes answer trust audit", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);

  assert.ok(payload.evidenceAudit);
  assert.match(payload.evidenceAudit.label, /复核|重查|引用/);
  assert.equal(payload.evidenceAudit.level, "medium");
  assert.ok(payload.evidenceAudit.counts.sourceEvidence >= 1);
  assert.ok(payload.evidenceAudit.checks.some((check) => check.id === "source_coverage"));
  assert.ok(payload.evidenceAudit.checks.some((check) => check.id === "claim_boundary"));
  assert.match(payload.evidenceAudit.checks.find((check) => check.id === "conflict_scan").message, /未发现明显冲突|不能证明.*没有冲突/);
});

test("buildQaPayload exposes local usage footprint and cloud cost boundary", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);

  assert.ok(payload.usageFootprint);
  assert.equal(payload.usageFootprint.mode, "template_fallback");
  assert.equal(payload.usageFootprint.cloudBillableTokens, 0);
  assert.match(payload.usageFootprint.summary, /云端计费 token 为 0/);
  assert.ok(payload.usageFootprint.estimate.totalCloudEquivalentTokens > 0);
  assert.match(payload.usageFootprint.boundary, /不是云模型账单/);
});

test("buildQaPayload can use a source-bound local model answer without dropping evidence markers", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT, undefined, {
    answerOverride: [
      "问题：主图视觉点击率转化率怎么优化？",
      "",
      "可执行结论：",
      "1. 先把主图当成点击入口核对，但不要脱离 Listing 承接判断。【证据1】",
      "",
      "执行顺序：",
      "1. 先看主图点击率，再看详情页转化承接。【证据2】",
      "",
      "建议下一步：打开来源核对原文，再补 CTR 和 CVR 数据。",
    ].join("\n"),
    answerGeneration: {
      mode: "local_ollama",
      model: "qwen2.5:3b",
      label: "本地模型来源回答",
    },
  });

  assert.match(payload.answer, /本地模型|主图当成点击入口|证据1/);
  assert.equal(payload.answerGeneration.mode, "local_ollama");
  assert.equal(payload.answerGeneration.model, "qwen2.5:3b");
  assert.equal(payload.usageFootprint.model, "qwen2.5:3b");
  assert.ok(payload.evidenceChain.claims.some((claim) => claim.type === "source_evidence"));
});

test("buildQaPayload allows high trust when sources are fresh, broad, and not conflicting", () => {
  const cleanContext = `Query: 主图点击率怎么优化？

飞翔的波波 2026-04-01 主图点击率: # 主图点击率
作者：飞翔的波波
发布时间：2026-04-01 08:00:00
原文链接：https://mp.weixin.qq.com/s/high-a
来源文件：飞翔的波波html/high-a.html
主图决定点击率，应该优先优化首图视觉表达。
点击率优化需要对比竞品主图、差异化卖点和价格带。
主图视觉影响广告点击效率。

张子卿 2026-04-02 主图视觉: # 主图视觉
作者：张子卿
发布时间：2026-04-02 08:00:00
原文链接：https://mp.weixin.qq.com/s/high-b
来源文件：张子卿html/high-b.html
首图视觉是点击入口，优化主图需要突出核心功能。
主图提升点击率，Listing 承接负责转化。
关键词和图片表达要一致。`;
  const payload = buildQaPayload("主图点击率怎么优化？", cleanContext);

  assert.equal(payload.evidenceAudit.level, "high");
  assert.match(payload.evidenceAudit.label, /引用支撑较充分/);
  assert.equal(payload.evidenceAudit.counts.conflictSignals, 0);
});

test("buildQaPayload keeps thin old single-source answers in low trust tier", () => {
  const oldSingleSource = `Query: 主图怎么优化？

跨境电商长期主义 2020-01-01 老资料: # 老资料
作者：跨境电商长期主义
发布时间：2020-01-01 08:00:00
原文链接：https://mp.weixin.qq.com/s/old
来源文件：跨境电商长期主义html/old.html
主图决定点击率，转化率还要看 Listing 文案、评价基础和广告流量质量。`;
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", oldSingleSource);

  assert.equal(payload.evidenceAudit.level, "low");
  assert.match(payload.evidenceAudit.label, /重查/);
  assert.match(payload.evidenceAudit.summary, /有限原文证据|重查/);
});

test("buildQaPayload surfaces possible conflicts across source evidence", () => {
  const conflictContext = `Query: 主图视觉点击率转化率怎么优化？

飞翔的波波 2026-04-01 主图先行法: # 主图先行法
作者：飞翔的波波
发布时间：2026-04-01 08:00:00
原文链接：https://mp.weixin.qq.com/s/conflict-a
来源文件：飞翔的波波html/conflict-a.html
主图是点击率的核心瓶颈，必须先优化主图视觉，因为首图极大程度决定点击率。

张子卿 2026-04-02 不要先改主图: # 不要先改主图
作者：张子卿
发布时间：2026-04-02 08:00:00
原文链接：https://mp.weixin.qq.com/s/conflict-b
来源文件：张子卿html/conflict-b.html
转化率问题不建议先改主图，主图不是当前瓶颈，应该先看评价、价格和页面承接。`;
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", conflictContext);
  const conflictCheck = payload.evidenceAudit.checks.find((check) => check.id === "conflict_scan");

  assert.equal(payload.evidenceAudit.level, "low");
  assert.ok(payload.evidenceAudit.counts.conflictSignals >= 1);
  assert.match(conflictCheck.message, /可能冲突|主图/);
  assert.match(payload.evidenceAudit.summary, /相反|人工对比/);
  assert.ok(conflictCheck.sourceIndexes.length >= 2);
  assert.ok(payload.evidenceAudit.conflictSignals.some((item) => item.concept === "主图"));
  const conflict = payload.evidenceAudit.conflictSignals.find((item) => item.concept === "主图");
  assert.ok(conflict.comparison);
  assert.match(conflict.comparison.summary, /主图/);
  assert.match(conflict.comparison.differenceFocus, /优先|先/);
  assert.match(conflict.comparison.suggestedCheck, /点击率|转化率|评价|价格|页面/);
  assert.ok(conflict.comparison.supportSource);
  assert.ok(conflict.comparison.cautionSource);
  assert.match(conflict.comparison.supportQuote, /必须先优化主图/);
  assert.match(conflict.comparison.cautionQuote, /不建议先改主图/);
  assert.ok(Number.isInteger(conflict.comparison.supportSource.sourceIndex));
  assert.ok(Number.isInteger(conflict.comparison.cautionSource.sourceIndex));
  assert.notEqual(conflict.comparison.supportSource.sourceIndex, conflict.comparison.cautionSource.sourceIndex);
  assert.equal(conflict.comparison.supportSource.author, "飞翔的波波");
  assert.equal(conflict.comparison.cautionSource.author, "张子卿");
  assert.equal(conflict.comparison.supportSource.date, "2026-04-01");
  assert.ok(conflict.comparison.supportSource.sourceUrl || conflict.comparison.supportSource.sourcePath);
  assert.ok(conflict.comparison.nextQuestion);
  assert.equal(conflict.comparison.nextQuestion.intent, "resolve_conflict");
  assert.match(conflict.comparison.nextQuestion.question, /主图/);
  assert.match(conflict.comparison.nextQuestion.question, /具体产品数据|优先级/);
  assert.ok(Array.isArray(conflict.comparison.nextQuestion.requiredData));
  assert.ok(conflict.comparison.nextQuestion.requiredData.some((item) => /点击率|CTR/.test(item.label)));
  assert.ok(conflict.comparison.nextQuestion.requiredData.some((item) => /转化率|CVR/.test(item.label)));
  assert.ok(conflict.comparison.nextQuestion.requiredData.some((item) => /价格/.test(item.label)));
  assert.ok(conflict.comparison.nextQuestion.requiredData.some((item) => /评价|评分/.test(item.label)));
  assert.ok(conflict.comparison.nextQuestion.requiredData.some((item) => /曝光|点击|流量/.test(item.label)));
  assert.equal(conflict.comparison.nextQuestion.evidenceRefs.supportSourceIndex, conflict.comparison.supportSource.sourceIndex);
  assert.equal(conflict.comparison.nextQuestion.evidenceRefs.cautionSourceIndex, conflict.comparison.cautionSource.sourceIndex);
  assert.notEqual(conflict.comparison.nextQuestion.evidenceRefs.supportSourceIndex, conflict.comparison.nextQuestion.evidenceRefs.cautionSourceIndex);
  assert.match(conflict.comparison.nextQuestion.boundary, /数据不完整|不能直接判断/);
});

test("buildQaPayload merges overlapping visual click conflict signals", () => {
  const conflictContext = `Query: 主图视觉点击率转化率怎么优化？

飞翔的波波 2026-04-01 主图点击先行: # 主图点击先行
作者：飞翔的波波
发布时间：2026-04-01 08:00:00
原文链接：https://mp.weixin.qq.com/s/merge-a
来源文件：飞翔的波波html/merge-a.html
主图是点击率的核心瓶颈，点击率低必须先优化主图视觉。

张子卿 2026-04-02 点击不要先改图: # 点击不要先改图
作者：张子卿
发布时间：2026-04-02 08:00:00
原文链接：https://mp.weixin.qq.com/s/merge-b
来源文件：张子卿html/merge-b.html
点击率低也不建议先改主图，主图不是当前瓶颈，应该先看评价、价格和页面承接。`;
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", conflictContext);
  const visualConflicts = payload.evidenceAudit.conflictSignals.filter((item) => ["主图", "点击率"].includes(item.concept));

  assert.equal(visualConflicts.length, 1);
  assert.equal(visualConflicts[0].concept, "主图");
  assert.deepEqual(visualConflicts[0].relatedConcepts, ["点击率"]);
  assert.match(payload.evidenceAudit.checks.find((check) => check.id === "conflict_scan").message, /主图\/点击率/);
  assert.equal(payload.evidenceAudit.counts.conflictSignals, 1);
});

test("buildQaPayload demotes conversion conflicts that only support visual-click decisions", () => {
  const conflictContext = `Query: 主图视觉点击率转化率怎么优化？

飞翔的波波 2026-04-01 主图点击先行: # 主图点击先行
作者：飞翔的波波
发布时间：2026-04-01 08:00:00
原文链接：https://mp.weixin.qq.com/s/supporting-a
来源文件：飞翔的波波html/supporting-a.html
主图是点击率和转化率的核心瓶颈，点击率低必须先优化主图视觉。

张子卿 2026-04-02 点击不要先改图: # 点击不要先改图
作者：张子卿
发布时间：2026-04-02 08:00:00
原文链接：https://mp.weixin.qq.com/s/supporting-b
来源文件：张子卿html/supporting-b.html
转化率问题不建议先改主图，主图不是当前瓶颈，应该先看评价、价格和流量质量。`;
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", conflictContext);
  const conflict = payload.evidenceAudit.conflictSignals[0];

  assert.equal(payload.evidenceAudit.conflictSignals.length, 1);
  assert.equal(payload.evidenceAudit.counts.conflictSignals, 1);
  assert.equal(payload.evidenceAudit.counts.supportingConflictReasons, 1);
  assert.equal(conflict.concept, "主图");
  assert.ok(conflict.relatedConcepts.includes("点击率"));
  assert.ok(conflict.supportingReasons.some((item) => item.concept === "转化率"));
  assert.match(conflict.supportingReasons[0].summary, /辅助|验证/);
  const ctrData = conflict.comparison.nextQuestion.requiredData.find((item) => item.id === "ctr");
  const cvrData = conflict.comparison.nextQuestion.requiredData.find((item) => item.id === "cvr");
  const priceData = conflict.comparison.nextQuestion.requiredData.find((item) => item.id === "price");
  const reviewData = conflict.comparison.nextQuestion.requiredData.find((item) => item.id === "reviews");
  assert.equal(ctrData.targetRole, "primary");
  assert.match(ctrData.verifies, /主图\/点击率|点击入口/);
  assert.equal(cvrData.targetRole, "supporting");
  assert.match(cvrData.verifies, /转化率|页面承接|辅助/);
  assert.equal(priceData.targetRole, "supporting");
  assert.match(priceData.verifies, /价格/);
  assert.equal(reviewData.targetRole, "supporting");
  assert.match(reviewData.verifies, /评价|信任/);
  assert.match(conflict.comparison.nextQuestion.question, /数据用途/);
  assert.match(conflict.comparison.nextQuestion.question, /主图点击率 \/ 广告 CTR：验证/);
  assert.match(conflict.comparison.nextQuestion.question, /转化率 \/ CVR：验证/);
  assert.match(payload.evidenceAudit.checks.find((check) => check.id === "conflict_scan").message, /辅助判断理由/);
});

test("buildQaPayload does not mark complementary visual advice as a conflict", () => {
  const complementaryContext = `Query: 主图视觉点击率转化率怎么优化？

跨境电商长期主义 2026-01-23 焦虑优化: # 焦虑优化
作者：跨境电商长期主义
发布时间：2026-01-23 08:00:00
原文链接：https://mp.weixin.qq.com/s/complement-a
来源文件：跨境电商长期主义html/complement-a.html
该梳理搜索词，重新梳理下搜索词，该优化文案视觉，就逼自己静下心来再多跑一遍，看看点击率，转化率的波动情况，寻找更好的转化区间。

跨境电商长期主义 2025-04-20 视觉包装: # 视觉包装
作者：跨境电商长期主义
发布时间：2025-04-20 08:00:00
原文链接：https://mp.weixin.qq.com/s/complement-b
来源文件：跨境电商长期主义html/complement-b.html
销售中其实有不少的问题是在选品，开发和立项阶段就已经埋下的祸根，有些小伙伴一开始并不注重产品的视觉，不注重包装，结果产品上架之后，因为包装稀烂，又缺乏有效的信息，导致无法通过短平快的方式抓住潜在客户的注意力，结果就是广告的点击率，转化率相比同行，简直可以用云泥之别来形容了。`;
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", complementaryContext);

  assert.equal(payload.evidenceAudit.counts.conflictSignals, 0);
  assert.equal(payload.evidenceAudit.conflictSignals.length, 0);
  assert.notEqual(payload.evidenceAudit.level, "low");
});

test("buildQaPayload keeps independent listing conversion conflicts", () => {
  const conflictContext = `Query: 转化率 Listing 应该先优化吗？

飞翔的波波 2026-04-01 页面转化先行: # 页面转化先行
作者：飞翔的波波
发布时间：2026-04-01 08:00:00
原文链接：https://mp.weixin.qq.com/s/listing-a
来源文件：飞翔的波波html/listing-a.html
Listing 页面承接是转化率的核心瓶颈，必须先优化副图、五点和 A+。

张子卿 2026-04-02 先别改页面: # 先别改页面
作者：张子卿
发布时间：2026-04-02 08:00:00
原文链接：https://mp.weixin.qq.com/s/listing-b
来源文件：张子卿html/listing-b.html
转化率不是当前瓶颈，不建议先改 Listing 页面，应该先看点击率、流量质量和价格。`;
  const payload = buildQaPayload("转化率 Listing 应该先优化吗？", conflictContext);
  const listingConflict = payload.evidenceAudit.conflictSignals.find((item) => item.concept === "Listing" || item.relatedConcepts?.includes("Listing"));

  assert.ok(listingConflict);
  assert.match([listingConflict.concept, ...(listingConflict.relatedConcepts || [])].join("/"), /Listing|转化率/);
  assert.match(listingConflict.comparison.nextQuestion.question, /页面|Listing|转化率/);
  assert.ok(listingConflict.comparison.nextQuestion.requiredData.some((item) => /副图|五点|A\+|页面/.test(item.label)));
  const cvrData = listingConflict.comparison.nextQuestion.requiredData.find((item) => item.id === "cvr");
  const listingContentData = listingConflict.comparison.nextQuestion.requiredData.find((item) => item.id === "listing_content");
  assert.equal(cvrData.targetRole, "primary");
  assert.match(cvrData.verifies, /转化率|Listing|第一优先级/);
  assert.equal(listingContentData.targetRole, "primary");
  assert.match(listingContentData.verifies, /转化率|Listing|第一优先级/);
});

test("buildQaPayload does not treat funnel boundary advice as a conflict", () => {
  const boundaryContext = `Query: 主图视觉点击率转化率怎么优化？

飞翔的波波 2026-04-01 主图点击: # 主图点击
作者：飞翔的波波
发布时间：2026-04-01 08:00:00
原文链接：https://mp.weixin.qq.com/s/boundary-a
来源文件：飞翔的波波html/boundary-a.html
主图决定点击率，应该优先优化首图视觉表达。

跨境电商长期主义 2026-04-02 转化承接: # 转化承接
作者：跨境电商长期主义
发布时间：2026-04-02 08:00:00
原文链接：https://mp.weixin.qq.com/s/boundary-b
来源文件：跨境电商长期主义html/boundary-b.html
转化率问题不要只改主图，还要看评价、价格和页面承接。`;
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", boundaryContext);

  assert.equal(payload.evidenceAudit.counts.conflictSignals, 0);
  assert.notEqual(payload.evidenceAudit.level, "low");
  assert.ok(payload.evidenceAudit.conflictSignals.every((item) => !item.comparison));
  assert.ok(payload.evidenceAudit.conflictSignals.every((item) => !item.comparison?.nextQuestion));
});

test("buildEvidenceChain marks missing source support when no sources exist", () => {
  const chain = buildEvidenceChain("没有资料的问题", "这次没有从本地知识库里找到足够相关的资料。", [], []);

  assert.equal(chain.claims.length, 1);
  assert.equal(chain.claims[0].type, "needs_source");
  assert.match(chain.claims[0].validation, /更具体|资料/);
});

test("buildQaPayload marks no-source answers inline", () => {
  const payload = buildQaPayload("完全没有命中的问题", "");

  assert.match(payload.answer, /【缺少来源】/);
  assert.equal(payload.evidenceChain.claims[0].type, "needs_source");
  assert.equal(payload.evidenceAudit.level, "low");
  assert.match(payload.evidenceAudit.summary, /缺少|资料/);
});

test("buildQaPayload exposes a knowledge gap radar for no-source answers", () => {
  const payload = buildQaPayload("完全没有命中的问题", "");

  assert.equal(payload.knowledgeGapRadar.status, "needs_source");
  assert.match(payload.knowledgeGapRadar.title, /知识缺口雷达/);
  assert.ok(payload.knowledgeGapRadar.gaps.some((gap) => gap.id === "gap:source"));
  assert.ok(payload.knowledgeGapRadar.gaps.every((gap) => gap.canUseAsEvidence === false));
  assert.match(payload.knowledgeGapRadar.boundary, /不改变作者原文证据边界/);
});

test("buildQaPayload uses the knowledge gap radar to choose the next source or data gap", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);

  assert.ok(payload.knowledgeGapRadar);
  assert.ok(["needs_review", "needs_data", "ready_to_validate"].includes(payload.knowledgeGapRadar.status));
  assert.ok(payload.knowledgeGapRadar.metrics.evidenceCount > 0);
  assert.ok(payload.knowledgeGapRadar.gaps.some((gap) => gap.id === "gap:author-view" || gap.id === "gap:business-data"));
  assert.ok(payload.knowledgeGapRadar.gaps.every((gap) => gap.canUseAsEvidence === false));
  assert.doesNotMatch(JSON.stringify(payload.knowledgeGapRadar), /canUseAsEvidence":true/);
});

test("buildQaPayload recommends the next source to read without promoting the recommendation to evidence", () => {
  const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT);

  assert.equal(payload.nextBestSource.title, "下一步资料选择");
  assert.equal(payload.nextBestSource.recommended.kind, "source");
  assert.equal(payload.nextBestSource.recommended.canUseAsEvidence, false);
  assert.equal(payload.nextBestSource.recommended.sourceCanUseAsEvidence, true);
  assert.equal(payload.nextBestSource.recommended.sourceIndex, 0);
  assert.ok(payload.nextBestSource.criteria.some((item) => item.id === "source-evidence"));
  assert.match(payload.nextBestSource.boundary, /推荐理由是系统整理，不是新的作者原文证据/);
  assert.doesNotMatch(JSON.stringify(payload.nextBestSource), /最优资料/);
});

test("buildQaPayload asks for source search when the next source cannot be grounded", () => {
  const payload = buildQaPayload("完全没有命中的问题", "");

  assert.equal(payload.nextBestSource.status, "needs_source");
  assert.equal(payload.nextBestSource.recommended.kind, "source_search");
  assert.equal(payload.nextBestSource.recommended.canUseAsEvidence, false);
  assert.equal(payload.nextBestSource.recommended.sourceCanUseAsEvidence, false);
  assert.match(payload.nextBestSource.recommended.reason, /作者原文/);
  assert.match(payload.nextBestSource.boundary, /不会改变本地知识库内容/);
});

test("buildKnowledgeHealthSummary surfaces missing semantic index", () => {
  const summary = buildKnowledgeHealthSummary({
    documents: 1779,
    chunks: 14597,
    embeddedChunks: 0,
    graphRelations: 2,
  });

  assert.equal(summary.vectorCoveragePercent, 0);
  assert.equal(summary.level, "needs_index");
  assert.match(summary.message, /语义索引还没有建立/);
});

test("buildKnowledgeHealthSummary keeps OpenHuman source tree status separate from vector coverage", () => {
  const summary = buildKnowledgeHealthSummary({
    documents: 1779,
    chunks: 14597,
    embeddedChunks: 14597,
    graphRelations: 2,
    sourceTree: {
      manifestDocuments: 1779,
      ingestedDocuments: 0,
      chunkSourceDocuments: 0,
      coveragePercent: 0,
      level: "empty",
      message: "作者原文还没有进入 OpenHuman 来源树。",
    },
  });

  assert.equal(summary.vectorCoveragePercent, 100);
  assert.equal(summary.sourceTree.level, "empty");
  assert.equal(summary.sourceTree.coveragePercent, 0);
  assert.match(summary.message, /来源树/);
});

test("buildKnowledgeReadinessSummary separates searchable, citable, and learnable states", () => {
  const readiness = buildKnowledgeReadinessSummary({
    manifestDocuments: 1779,
    storedDocuments: 1779,
    health: buildKnowledgeHealthSummary({
      documents: 1779,
      chunks: 14597,
      embeddedChunks: 14597,
      graphRelations: 2,
      sourceTree: {
        manifestDocuments: 1779,
        ingestedDocuments: 413,
        chunkSourceDocuments: 413,
        chunks: 2101,
        trees: 3,
        summaries: 0,
        readyJobs: 2101,
        runningJobs: 0,
        failedJobs: 0,
        queuedJobs: 2101,
        coveragePercent: 23.2,
        level: "processing",
        message: "OpenHuman 来源树正在处理。",
      },
    }),
  });

  assert.equal(readiness.level, "answer_ready_learning_processing");
  assert.equal(readiness.answerStatus, "ready");
  assert.deepEqual(readiness.stages.map((stage) => stage.id), ["search", "citation", "learning"]);
  assert.equal(readiness.stages[0].status, "ready");
  assert.match(readiness.stages[0].detail, /14597\/14597/);
  assert.equal(readiness.stages[1].status, "ready");
  assert.match(readiness.stages[1].detail, /1779\/1779/);
  assert.equal(readiness.stages[2].status, "processing");
  assert.match(readiness.stages[2].detail, /413\/1779/);
  assert.match(readiness.message, /可问答/);
  assert.match(readiness.message, /可引用/);
  assert.match(readiness.message, /后台深加工/);
});

function normalizeEvidenceTextForTest(value) {
  return String(value || "")
    .replace(/[【】\[\]（）()《》「」“”"'`]/g, "")
    .replace(/\s+/g, "")
    .trim();
}
