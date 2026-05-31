import assert from "node:assert/strict";

import { buildQaPayload } from "./amazon-qa-lib.mjs";

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

const PRODUCT_CONTEXT = `Query: 新品选品应该如何判断是否值得做？

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

const WEAK_CONTEXT = `Query: 主图视觉点击率转化率怎么优化？

张子卿 2024-11-14 亚马逊实盘记录45 —— 平行兼容＞向上社交: # 亚马逊实盘记录45 —— 平行兼容＞向上社交
作者：张子卿
发布时间：2024-11-14 17:35:37
原文链接：https://mp.weixin.qq.com/s/weak
来源文件：张子卿html/weak.html
很多人问主图这个事情，但这篇文章主要是在复盘个人状态、团队协作和长期写作节奏，不讨论点击率、转化率、Listing 或亚马逊页面优化。`;

const WORKFLOW_MEMORY_RESULT = {
  data: {
    context: {
      chunks: [
        {
          content: `# 亚马逊学习档案：视觉转化诊断

## 问题
主图视觉点击率转化率怎么优化？

## 当前结论
先把主图当成点击率入口处理；没有点击率，后面的转化率分析意义会变弱。`,
          metadata: {
            namespace: "amazon-learning-workflow",
            key: "dossier/adversarial-workflow",
            source_type: "amazon-learning-dossier",
            title: "亚马逊学习档案：视觉转化诊断",
          },
        },
      ],
    },
  },
};

function sourceEvidence(payload) {
  return (payload.evidenceChain?.claims || []).filter((claim) => claim.type === "source_evidence");
}

const cases = [
  {
    name: "weak_related_article_refused",
    run() {
      const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", WEAK_CONTEXT);
      assert.equal(payload.sources.length, 0);
      assert.equal(payload.validationPack.status, "needs_source");
      assert.equal(sourceEvidence(payload).length, 0);
      assert.doesNotMatch(payload.answer, /【证据\d+】/);
    },
  },
  {
    name: "workflow_memory_not_source",
    run() {
      const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", WORKFLOW_MEMORY_RESULT);
      assert.equal(payload.sources.length, 0);
      assert.equal(sourceEvidence(payload).length, 0);
      assert.equal(payload.sourceStudyPack.status, "needs_source");
    },
  },
  {
    name: "out_of_domain_retrieval_refused",
    run() {
      const payload = buildQaPayload("量子芯片低温纠错架构怎么搭？", MAIN_IMAGE_CONTEXT);
      assert.equal(payload.sources.length, 0);
      assert.equal(sourceEvidence(payload).length, 0);
      assert.equal(payload.validationPack.status, "needs_source");
    },
  },
  {
    name: "allowed_author_only",
    run() {
      const payload = buildQaPayload("新品选品应该如何判断是否值得做？", PRODUCT_CONTEXT, "新品选品应该如何判断是否值得做？", {
        allowedAuthors: ["飞翔的波波"],
      });
      assert.ok(payload.sources.length > 0);
      assert.ok(payload.sources.every((source) => source.author === "飞翔的波波"));
      assert.ok(sourceEvidence(payload).every((claim) => claim.author === "飞翔的波波"));
    },
  },
  {
    name: "selected_source_missing_no_fallback",
    run() {
      const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT, "主图视觉点击率转化率怎么优化？", {
        allowedSourceKeys: ["不存在的来源"],
      });
      assert.equal(payload.sources.length, 0);
      assert.equal(payload.sourceScope.totalAfterScope, 0);
      assert.equal(payload.validationPack.status, "needs_source");
    },
  },
  {
    name: "excluded_source_not_reused",
    run() {
      const excludedKey = "跨境电商长期主义|2022-06-18|你是如何解决转化率的？";
      const payload = buildQaPayload("主图视觉点击率转化率怎么优化？", MAIN_IMAGE_CONTEXT, "主图视觉点击率转化率怎么优化？", {
        excludedSourceKeys: [excludedKey],
      });
      assert.ok(payload.sources.length > 0);
      assert.ok(payload.sources.every((source) => source.title !== "你是如何解决转化率的？"));
      assert.ok(sourceEvidence(payload).every((claim) => claim.title !== "你是如何解决转化率的？"));
    },
  },
];

let failed = 0;
for (const item of cases) {
  try {
    item.run();
    console.log(`PASS ${item.name}`);
  } catch (error) {
    failed += 1;
    console.error(`FAIL ${item.name}`);
    console.error(error?.stack || error);
  }
}

console.log(`\n${cases.length - failed}/${cases.length} adversarial checks passed`);
if (failed > 0) process.exitCode = 1;
