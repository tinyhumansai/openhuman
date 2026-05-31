import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import vm from "node:vm";

const html = fs.readFileSync(new URL("./amazon-qa-ui.html", import.meta.url), "utf8");
const serverSource = fs.readFileSync(new URL("./amazon-qa-server.mjs", import.meta.url), "utf8");
const drainRunnerSource = fs.readFileSync(new URL("./amazon-source-tree-drain-runner.mjs", import.meta.url), "utf8");

test("amazon QA UI inline script remains syntactically valid", () => {
  const scripts = [...html.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/gi)].map((match) => match[1]);
  assert.ok(scripts.length > 0);
  scripts.forEach((script, index) => {
    assert.doesNotThrow(() => new vm.Script(script, { filename: `amazon-qa-ui.inline-${index + 1}.js` }));
  });
});

test("conflict data section includes structured data collection flow", () => {
  assert.match(html, /conflict-data-form/);
  assert.match(html, /data-conflict-field-label/);
  assert.match(html, /生成数据追问/);
  assert.match(html, /bindStructuredDataFormEvents/);
  assert.match(html, /buildConflictDataQuestion/);
  assert.match(html, /queueProductInputForPrompt\(question,\s*productText,\s*null,\s*\{/);
});

test("sidebar exposes knowledge readiness instead of only raw document count", () => {
  assert.match(html, /knowledgeReadiness/);
  assert.match(html, /renderKnowledgeReadiness/);
  assert.match(html, /renderReadinessStages/);
  assert.match(html, /可问答就绪状态/);
  assert.match(html, /可搜索/);
  assert.match(html, /可引用/);
  assert.match(html, /可学习/);
  assert.match(html, /可回答证据/);
  assert.match(html, /OpenHuman 来源树/);
  assert.match(html, /status\.readiness/);
  assert.match(html, /sourceTree/);
  assert.match(html, /来源树原文覆盖/);
  assert.match(html, /全库图谱很薄/);
  assert.match(html, /renderSourceTreeDrainControls/);
  assert.match(html, /startSourceTreeDrain/);
  assert.match(html, /stopSourceTreeDrain/);
  assert.match(html, /\/api\/source-tree-drain\/start/);
  assert.match(html, /\/api\/source-tree-drain\/stop/);
  assert.match(html, /来源树深加工/);
  assert.match(html, /跑 10 个/);
  assert.match(html, /跑 50 个/);
  assert.match(html, /跑 250 个/);
  assert.match(html, /data-source-tree-drain-jobs/);
  assert.match(html, /data-source-tree-drain-batch-size/);
  assert.match(html, /data-source-tree-drain-sleep-ms/);
  assert.doesNotMatch(html, /data-source-tree-drain-full/);
  assert.doesNotMatch(html, /跑到完成/);
  assert.match(html, /暂停批次/);
  assert.match(html, /每次只跑一个有限批次/);
  assert.match(html, /跑完自动暂停/);
  assert.match(html, /sourceTreeDrainStartPayload/);
  assert.match(html, /clampSourceTreeDrainNumber/);
  assert.match(html, /maxJobs,\s*batchSize,\s*sleepMs/);
  assert.doesNotMatch(html, /runToComplete/);
  assert.doesNotMatch(html, /batchSize: runToComplete/);
  assert.match(html, /clampSourceTreeDrainNumber\(button\?\.dataset\?\.sourceTreeDrainJobs,\s*10,\s*1,\s*250\)/);
  assert.match(html, /队列：可处理/);
  assert.match(html, /上一批实际处理/);
  assert.match(html, /当前等待数以实时数据库为准/);
  assert.match(html, /queuedDelta/);
  assert.match(html, /doneDelta/);
  assert.match(html, /formatBatchQueueDelta/);
  assert.match(html, /预计剩余/);
  assert.match(html, /速度/);
  assert.match(html, /formatDurationMinutes/);
  assert.match(html, /setInterval\(\(\) =>/);
  assert.match(html, /日志：/);
});

test("source-tree drain start remains finite from the browser and server entrypoint", () => {
  assert.doesNotMatch(html, /maxJobs:\s*0/);
  assert.match(html, /maxJobs,\s*batchSize,\s*sleepMs/);
  assert.match(serverSource, /clampNumber\(body\?\.maxJobs,\s*250,\s*1,\s*250\)/);
  assert.doesNotMatch(serverSource, /clampNumber\(body\?\.maxJobs,\s*250,\s*0,\s*5000\)/);
  assert.match(drainRunnerSource, /clampNumberArg\(argValue\(args,\s*"--max-jobs",\s*"250"\),\s*250,\s*1,\s*250\)/);
  assert.doesNotMatch(drainRunnerSource, /maxJobs === 0/);
});

test("sidebar lets users add and scope their own sources", () => {
  assert.match(html, /我的资料/);
  assert.match(html, /userSourceTitle/);
  assert.match(html, /userSourceContent/);
  assert.match(html, /userSourceOnlyMode/);
  assert.match(html, /\/api\/user-sources/);
  assert.match(html, /userSourceControls/);
  assert.match(html, /本轮只问已勾选的我的资料/);
});

test("sidebar exposes a learning note workspace without treating notes as author evidence", () => {
  assert.match(html, /学习笔记/);
  assert.match(html, /learningNoteTitle/);
  assert.match(html, /learningNoteContent/);
  assert.match(html, /saveLearningNoteButton/);
  assert.match(html, /learningNotesEl/);
  assert.match(html, /\/api\/notes/);
  assert.match(html, /saveManualLearningNote/);
  assert.match(html, /saveAnswerAsLearningNote/);
  assert.match(html, /answerLearningNoteContent/);
  assert.match(html, /data-answer-note-save/);
  assert.match(html, /data-note-to-user-source/);
  assert.match(html, /转成我的资料/);
  assert.match(html, /笔记不会自动变成作者证据/);
  assert.match(html, /不会写入作者原文证据库/);
  assert.doesNotMatch(html, /learning-note-item[\s\S]{0,1200}data-dossier-source-decision/);
});

test("assistant answers expose one primary next step action", () => {
  assert.match(html, /renderNextStepPanel/);
  assert.match(html, /bindNextStepEvents/);
  assert.match(html, /本轮下一步/);
  assert.match(html, /data-next-step-action/);
  assert.match(html, /先核对关键来源/);
});

test("assistant answers expose a per-answer learning queue with persistent progress", () => {
  assert.match(html, /renderLearningQueue/);
  assert.match(html, /renderLearningQueueItem/);
  assert.match(html, /bindLearningQueueEvents/);
  assert.match(html, /updateLearningQueueItemState/);
  assert.match(html, /本轮学习队列/);
  assert.match(html, /data-learning-queue-action/);
  assert.match(html, /data-learning-queue-toggle/);
  assert.match(html, /标记看过/);
  assert.match(html, /取消标记/);
  assert.match(html, /本轮队列已全部标记/);
  assert.match(html, /compactLearningQueue/);
  assert.match(html, /learningQueue:\s*payload\.learningQueue/);
  assert.match(html, /learningQueue:\s*compactLearningQueue\(message\.learningQueue\)/);
});

test("sidebar exposes restorable topic notebooks and preserves answer graphs", () => {
  assert.match(html, /专题会话/);
  assert.match(html, /refreshNotebooksButton/);
  assert.match(html, /notebooksEl/);
  assert.match(html, /loadNotebooks/);
  assert.match(html, /restoreNotebookSession/);
  assert.match(html, /\/api\/notebooks/);
  assert.match(html, /data-notebook-open/);
  assert.match(html, /updateSessionUrl/);
  assert.match(html, /payload\.notebook/);
  assert.match(html, /upsertNotebook/);
  assert.match(html, /graph:\s*compactGraph\(message\.graph\)/);
  assert.match(html, /专题会话是系统整理，不是作者原文证据/);
});

test("topic notebooks can render a source-bound study pack", () => {
  assert.match(html, /notebookStudyPack/);
  assert.match(html, /loadNotebookStudyPack/);
  assert.match(html, /renderNotebookStudyPack/);
  assert.match(html, /\/api\/notebooks\/\$\{encodeURIComponent\(id\)\}\/study-pack/);
  assert.match(html, /data-notebook-study-pack/);
  assert.match(html, /专题学习包/);
  assert.match(html, /来源账本/);
  assert.match(html, /复制 Markdown/);
  assert.match(html, /下载 Markdown/);
  assert.match(html, /下载 JSON/);
  assert.match(html, /Studio 产物包/);
  assert.match(html, /renderStudyPackStudio/);
  assert.match(html, /renderStudyPackStudioReport/);
  assert.match(html, /renderStudyPackStudioActionPlan/);
  assert.match(html, /renderStudyPackStudioFlashcards/);
  assert.match(html, /renderStudyPackStudioMindMap/);
  assert.match(html, /renderStudyPackStudioSourceTable/);
  assert.match(html, /renderStudyPackStudioMastery/);
  assert.match(html, /下载复习卡 CSV/);
  assert.match(html, /下载来源表 CSV/);
  assert.match(html, /复习卡预览/);
  assert.match(html, /掌握度自测/);
  assert.match(html, /亚马逊行动实验计划/);
  assert.match(html, /行动实验计划/);
  assert.match(html, /来源到业务验证/);
  assert.match(html, /先核对来源，再补业务数据/);
  assert.match(html, /data-study-pack-mastery/);
  assert.match(html, /STUDY_PACK_MASTERY_KEY/);
  assert.match(html, /studyPackMasteryItemKey/);
  assert.match(html, /studyPackTextFingerprint/);
  assert.match(html, /pruneStudyPackMasteryState/);
  assert.match(html, /自测只检查你对本专题的理解/);
  assert.match(html, /思维导图预览/);
  assert.match(html, /来源数据表/);
  assert.match(html, /downloadStudyPackFile/);
  assert.match(html, /flashcardsCsv/);
  assert.match(html, /sourceTableCsv/);
  assert.match(html, /exportMarkdown/);
  assert.match(html, /exportJson/);
  assert.match(html, /study-pack-boundary/);
  assert.match(html, /系统整理，不是作者原文证据/);
  assert.match(html, /概念来自本轮问答图谱/);
  assert.match(html, /data-study-pack-prompt/);
  assert.doesNotMatch(html, /study-pack-mastery[\s\S]{0,1800}saveLearningNote/);
  assert.doesNotMatch(html, /study-pack-mastery[\s\S]{0,1800}data-dossier-source-decision/);
});

test("learning queue actions reuse existing reversible answer flows", () => {
  assert.match(html, /handleLearningQueueAction/);
  assert.match(html, /isUser \|\| message\.learningQueue \? "" : renderNextStepPanel/);
  assert.match(html, /setLearningQueueItemDone\(messageIndex,\s*"queue:dossier",\s*true\)/);
  assert.match(html, /evidenceGateStatus/);
  assert.match(html, /applyEvidenceGateFeedback/);
  assert.match(html, /learningDossierSaveGate/);
  assert.match(html, /messageNeedsSourceBeforeSave/);
  assert.match(html, /completionMode === "evidence_feedback"/);
  assert.match(html, /用证据反馈完成/);
  assert.match(html, /先核对证据/);
  assert.match(html, /先补来源/);
  assert.match(html, /item\.done = false/);
  assert.match(html, /saveGate\.locked/);
  assert.match(html, /focusClaimByKey\(button\.dataset\.claimTarget\)/);
  assert.match(html, /focusFirstStructuredDataField\(messageEl\)/);
  assert.match(html, /saveLearningNote\(Number\(button\.dataset\.messageIndex\)\)/);
  assert.match(html, /fillQuestionFromWorkflowPrompt\(button\.dataset\.learningQueuePrompt/);
  assert.doesNotMatch(html, /data-learning-queue-action[\s\S]{0,500}saveDossierSourceDecision/);
});

test("assistant answers expose a source-first learning studio without auto-promoting evidence", () => {
  assert.match(html, /renderAnswerStudio/);
  assert.match(html, /buildAnswerStudioItems/);
  assert.match(html, /本轮学习入口/);
  assert.match(html, /来源阅读会话/);
  assert.match(html, /先读来源并标记证据/);
  assert.match(html, /action:\s*"read-source"/);
  assert.match(html, /feedback:\s*"useful"/);
  assert.match(html, /feedback:\s*"irrelevant"/);
  assert.match(html, /bindAnswerStudioEvents/);
  assert.match(html, /focusAnswerStudioTarget/);
  assert.match(html, /updateEvidenceFeedback\(messageIndex,\s*button\.dataset\.answerStudioClaimId/);
  assert.match(html, /saveLearningNote\(messageIndex\)/);
  assert.doesNotMatch(html, /answer-studio[\s\S]{0,1400}data-dossier-source-decision/);
});

test("assistant answers expose a default one-page learning brief", () => {
  assert.match(html, /renderOnePageLearningBrief/);
  assert.match(html, /buildOnePageLearningBriefBlocks/);
  assert.match(html, /一页式学习简报/);
  assert.match(html, /本轮结论/);
  assert.match(html, /关键证据/);
  assert.match(html, /不能确认/);
  assert.match(html, /下一步动作/);
  assert.match(html, /answer-brief-grid/);
  assert.match(html, /renderAnswerStudioAction/);
  assert.match(html, /action:\s*"read-source"/);
  assert.match(html, /action:\s*"focus"/);
  assert.match(html, /data-kind="gap"/);
  assert.doesNotMatch(html, /answer-brief[\s\S]{0,1600}saveLearningNote/);
  assert.doesNotMatch(html, /answer-brief[\s\S]{0,1600}data-dossier-source-decision/);
});

test("answer graph supports node-driven follow-up questions like a learning map", () => {
  assert.match(html, /graphPromptForNode/);
  assert.match(html, /data-graph-prompt/);
  assert.match(html, /graph-prompt-button/);
  assert.match(html, /fillQuestionFromWorkflowPrompt\(button\.dataset\.graphPrompt/);
  assert.match(html, /fillQuestionFromWorkflowPrompt\(graphPrompt\)/);
  assert.match(html, /node\.type === "source" \|\| node\.type === "evidence"/);
  assert.match(html, /if \(type === "point"\) return "答案要点";/);
  assert.match(html, /if \(type === "step"\) return "执行步骤";/);
  assert.match(html, /if \(type === "concept"\) return "相关概念";/);
  assert.match(html, /点击继续问/);
  assert.doesNotMatch(html, /答案要点，点击继续问/);
  assert.doesNotMatch(html, /执行步骤，点击继续问/);
  assert.doesNotMatch(html, /相关概念，点击继续问/);
});

test("assistant answers expose a business validation task pack", () => {
  assert.match(html, /renderValidationPack/);
  assert.match(html, /bindValidationPackEvents/);
  assert.match(html, /本轮业务验证任务包/);
  assert.match(html, /data-validation-prompt/);
  assert.match(html, /message\.validationPack/);
});

test("business validation pack exposes product-data decision without promoting evidence", () => {
  assert.match(html, /renderBusinessDecision/);
  assert.match(html, /当前产品判断/);
  assert.match(html, /data-business-decision-status/);
  assert.match(html, /支持这个判断的数据/);
  assert.match(html, /仍缺的数据/);
  assert.match(html, /用户产品数据不是作者原文证据/);
  assert.match(html, /businessDecision:\s*compactBusinessDecision\(pack\.businessDecision\)/);
  assert.doesNotMatch(html, /business-decision[\s\S]{0,900}data-dossier-source-decision/);
  assert.doesNotMatch(html, /business-decision[\s\S]{0,900}saveLearningNote/);
});

test("assistant answers expose workflow intent routing", () => {
  assert.match(html, /renderWorkflowIntent/);
  assert.match(html, /message\.workflowIntent/);
  assert.match(html, /本轮意图/);
  assert.match(html, /data-workflow-intent/);
  assert.match(html, /buildWorkflowIntentActions/);
  assert.match(html, /data-workflow-action/);
  assert.match(html, /定位关键证据/);
  assert.match(html, /补产品数据/);
  assert.match(html, /看验证任务/);
  assert.match(html, /按上一问重查/);
  assert.match(html, /换具体问法/);
  assert.match(html, /用户已确认/);
  assert.match(html, /方法学习/);
  assert.match(html, /产品诊断/);
  assert.match(html, /实验复盘/);
});

test("learning cards expose source-backed understanding checks without auto-saving evidence", () => {
  assert.match(html, /renderStudyChecks/);
  assert.match(html, /本轮理解检查/);
  assert.match(html, /默认折叠/);
  assert.match(html, /data-study-check-prompt/);
  assert.match(html, /理解检查只用于学习复盘/);
  assert.match(html, /focusSourceByKey/);
  assert.doesNotMatch(html, /study-check[\s\S]{0,500}saveLearningNote/);
  assert.doesNotMatch(html, /study-check[\s\S]{0,500}data-dossier-source-decision/);
});

test("assistant answers expose a gated source study pack without auto-promoting evidence", () => {
  assert.match(html, /renderSourceStudyPack/);
  assert.match(html, /acceptedSourceStudyCards/);
  assert.match(html, /本轮来源研读包/);
  assert.match(html, /先核对证据/);
  assert.match(html, /作者原文/);
  assert.match(html, /系统整理/);
  assert.match(html, /sourceStudyPack:\s*payload\.sourceStudyPack/);
  assert.match(html, /sourceStudyPack:\s*compactSourceStudyPack\(message\.sourceStudyPack\)/);
  assert.match(html, /data-source-study-prompt/);
  assert.match(html, /data-source-target/);
  assert.doesNotMatch(html, /source-study[\s\S]{0,900}saveLearningNote/);
  assert.doesNotMatch(html, /source-study[\s\S]{0,900}data-dossier-source-decision/);
});

test("assistant answers expose a per-answer topic source tree and preserve it in history", () => {
  assert.match(html, /renderTopicSourceTree/);
  assert.match(html, /本轮主题来源树/);
  assert.match(html, /allSources\.slice\(0,\s*6\)/);
  assert.match(html, /allAuthors\.slice\(0,\s*4\)/);
  assert.match(html, /allPaths\.slice\(0,\s*6\)/);
  assert.match(html, /已展开/);
  assert.match(html, /data-topic-tree-source-target/);
  assert.match(html, /data-topic-tree-prompt/);
  assert.match(html, /topicSourceTree:\s*payload\.topicSourceTree/);
  assert.match(html, /topicSourceTree:\s*compactTopicSourceTree\(message\.topicSourceTree\)/);
  assert.match(html, /bindTopicSourceTreeEvents/);
  assert.match(html, /只基于本轮可引用的作者原文/);
  assert.doesNotMatch(html, /topic-source-tree[\s\S]{0,900}saveLearningNote/);
  assert.doesNotMatch(html, /topic-source-tree[\s\S]{0,900}data-dossier-source-decision/);
});

test("assistant answers expose a source-bound synthesis panel", () => {
  assert.match(html, /renderSynthesisAnswer/);
  assert.match(html, /本轮综合答案/);
  assert.match(html, /系统综合，不是新的作者原文证据/);
  assert.match(html, /synthesisAnswer:\s*payload\.synthesisAnswer/);
  assert.match(html, /synthesisAnswer:\s*compactSynthesisAnswer\(message\.synthesisAnswer\)/);
  assert.match(html, /data-synthesis-source-target/);
  assert.match(html, /data-source-context-target/);
  assert.doesNotMatch(html, /synthesis-answer[\s\S]{0,900}saveLearningNote/);
  assert.doesNotMatch(html, /synthesis-answer[\s\S]{0,900}data-dossier-source-decision/);
});

test("assistant answers expose a source trust chain with explicit evidence classes", () => {
  assert.match(html, /renderSourceTrustCard/);
  assert.match(html, /sourceTrust:\s*payload\.sourceTrust/);
  assert.match(html, /sourceTrust:\s*compactSourceTrust\(message\.sourceTrust\)/);
  assert.match(html, /本轮来源可信链路/);
  assert.match(html, /作者原文证据/);
  assert.match(html, /二次摘要\/系统整理/);
  assert.match(html, /用户产品材料/);
  assert.match(html, /实验\/复盘/);
  assert.match(html, /不足以确认/);
  assert.match(html, /有来源不等于已验证/);
  assert.match(html, /用户产品材料不是作者原文证据/);
  assert.match(html, /实验复盘不是作者原文证据/);
  assert.doesNotMatch(html, /source-trust[\s\S]{0,1200}data-dossier-source-decision/);
  assert.doesNotMatch(html, /source-trust[\s\S]{0,1200}saveLearningNote/);
});

test("assistant answers expose and preserve a knowledge gap radar", () => {
  assert.match(html, /renderKnowledgeGapRadar/);
  assert.match(html, /bindKnowledgeGapRadarEvents/);
  assert.match(html, /compactKnowledgeGapRadar/);
  assert.match(html, /knowledgeGapRadar:\s*payload\.knowledgeGapRadar/);
  assert.match(html, /knowledgeGapRadar:\s*compactKnowledgeGapRadar\(message\.knowledgeGapRadar\)/);
  assert.match(html, /知识缺口雷达/);
  assert.match(html, /data-knowledge-gap-prompt/);
  assert.match(html, /fillQuestionFromWorkflowPrompt\(button\.dataset\.knowledgeGapPrompt/);
  assert.match(html, /不改变作者原文证据边界/);
  assert.match(html, /学习提示，不作为新证据/);
  assert.doesNotMatch(html, /knowledge-gap-radar[\s\S]{0,1200}saveLearningNote/);
  assert.doesNotMatch(html, /knowledge-gap-radar[\s\S]{0,1200}data-dossier-source-decision/);
});

test("assistant answers expose a result confirmation loop after each answer", () => {
  assert.match(html, /renderAnswerEffectiveness/);
  assert.match(html, /bindAnswerEffectivenessEvents/);
  assert.match(html, /updateAnswerEffectiveness/);
  assert.match(html, /learningEffectivenessSummary/);
  assert.match(html, /renderLearningEffectivenessSummary/);
  assert.match(html, /buildLearningEffectivenessSummary/);
  assert.match(html, /bindLearningEffectivenessSummaryEvents/);
  assert.match(html, /focusMessageSection/);
  assert.match(html, /answerEffectiveness:\s*compactAnswerEffectiveness\(message\.answerEffectiveness\)/);
  assert.match(html, /answerEffectiveness:\s*payload\.answerEffectiveness/);
  assert.match(html, /syncAnswerEffectiveness/);
  assert.match(html, /syncMessageFeedback/);
  assert.match(html, /\/api\/notebooks/);
  assert.match(html, /answer-effectiveness/);
  assert.match(html, /message-feedback/);
  assert.match(html, /evidenceFeedback:\s*message\.evidenceFeedback/);
  assert.match(html, /encodeURIComponent\(sessionId\)/);
  assert.match(html, /本轮结果确认/);
  assert.match(html, /这次有效/);
  assert.match(html, /需要补来源/);
  assert.match(html, /切换意图/);
  assert.match(html, /补产品数据/);
  assert.match(html, /data-answer-effectiveness/);
  assert.match(html, /学习闭环状态/);
  assert.match(html, /确认最近回答/);
  assert.match(html, /data-learning-effectiveness-scroll/);
  assert.match(html, /data-learning-effectiveness-prompt/);
  assert.match(html, /不会把回答自动变成作者原文证据/);
  assert.match(html, /fillQuestionFromWorkflowPrompt\(prompt\)/);
  assert.match(html, /不会把回答自动变成作者原文证据/);
  assert.doesNotMatch(html, /answer-effectiveness[\s\S]{0,1200}saveLearningNote/);
  assert.doesNotMatch(html, /answer-effectiveness[\s\S]{0,1200}data-dossier-source-decision/);
  assert.doesNotMatch(html, /learning-effectiveness-summary[\s\S]{0,1600}data-dossier-source-decision/);
});

test("assistant answers expose and preserve the next source choice without overclaiming evidence", () => {
  assert.match(html, /renderNextBestSource/);
  assert.match(html, /bindNextBestSourceEvents/);
  assert.match(html, /compactNextBestSource/);
  assert.match(html, /nextBestSource:\s*payload\.nextBestSource/);
  assert.match(html, /nextBestSource:\s*compactNextBestSource\(message\.nextBestSource\)/);
  assert.match(html, /下一步资料选择/);
  assert.match(html, /data-next-best-source-prompt/);
  assert.match(html, /fillQuestionFromWorkflowPrompt\(button\.dataset\.nextBestSourcePrompt/);
  assert.match(html, /推荐理由是系统整理，不是新的作者原文证据/);
  assert.match(html, /资料本身可回到来源核对；本推荐理由不是作者证据/);
  assert.doesNotMatch(html, /最优资料/);
  assert.doesNotMatch(html, /next-best-source[\s\S]{0,1200}saveLearningNote/);
  assert.doesNotMatch(html, /next-best-source[\s\S]{0,1200}data-dossier-source-decision/);
});

test("composer can inspect the best source scope before asking", () => {
  assert.match(html, /questionSourceSelection/);
  assert.match(html, /sourceSelectionButton/);
  assert.match(html, /inspectQuestionSourceSelection/);
  assert.match(html, /\/api\/source-selection/);
  assert.match(html, /renderQuestionSourceSelection/);
  assert.match(html, /renderQuestionIntentChoice/);
  assert.match(html, /selectQuestionIntent/);
  assert.match(html, /applyQuestionSourceSelection/);
  assert.match(html, /问前资料选择/);
  assert.match(html, /问前意图确认/);
  assert.match(html, /data-question-intent/);
  assert.match(html, /intentPreference/);
  assert.match(html, /先选资料/);
  assert.match(html, /用这些资料提问/);
  assert.match(html, /候选理由是系统整理，不是新的作者原文证据/);
  assert.match(html, /sourceControls = normalizeSourceControls\(selection\.sourceControls\)/);
  assert.doesNotMatch(html, /question-source-selection[\s\S]{0,1200}saveLearningNote/);
  assert.doesNotMatch(html, /question-source-selection[\s\S]{0,1200}data-dossier-source-decision/);
});

test("source-tree calibration is shown as a routing hint and preserved in history", () => {
  assert.match(html, /sourceTreeCalibration:\s*payload\.sourceTreeCalibration/);
  assert.match(html, /sourceTreeCalibration:\s*compactSourceTreeCalibration\(message\.sourceTreeCalibration\)/);
  assert.match(html, /renderSourceTreeRoute/);
  assert.match(html, /source-trust-route/);
  assert.match(html, /不能当成作者原文证据/);
});

test("assistant answers expose a notebook-style study brief without auto-saving evidence", () => {
  assert.match(html, /renderNotebookGuide/);
  assert.match(html, /notebookGuide:\s*payload\.notebookGuide/);
  assert.match(html, /notebookGuide:\s*compactNotebookGuide\(message\.notebookGuide\)/);
  assert.match(html, /本轮学习简报/);
  assert.match(html, /data-notebook-guide-source-target/);
  assert.match(html, /data-notebook-guide-prompt/);
  assert.match(html, /系统整理，不是作者原文证据/);
  assert.doesNotMatch(html, /notebook-guide[\s\S]{0,1200}saveLearningNote/);
  assert.doesNotMatch(html, /notebook-guide[\s\S]{0,1200}data-dossier-source-decision/);
});

test("source references can open local original context without promoting evidence", () => {
  assert.match(html, /\/api\/source-context/);
  assert.match(html, /renderSourceContextPanel/);
  assert.match(html, /openSourceContext/);
  assert.match(html, /data-source-context-target/);
  assert.match(html, /原文上下文/);
  assert.match(html, /未定位到原文上下文/);
  assert.match(html, /已定位引用位置/);
  assert.match(html, /请核对上下文是否支持该结论/);
  assert.match(html, /作者原文/);
  assert.doesNotMatch(html, /原文上下文[\s\S]{0,500}已确认结论/);
  assert.doesNotMatch(html, /source-context[\s\S]{0,1200}saveLearningNote/);
  assert.doesNotMatch(html, /source-context[\s\S]{0,1200}data-dossier-source-decision/);
});

test("author perspective comparison is conflict-gated and source-boundary safe", () => {
  assert.match(html, /renderAuthorPerspectiveRoom/);
  assert.match(html, /跨作者观点对照/);
  assert.match(html, /只在真实冲突或明确对比问题中显示/);
  assert.match(html, /data-author-perspective-prompt/);
  assert.match(html, /data-source-context-target/);
  assert.match(html, /authorPerspectiveRoom:\s*payload\.authorPerspectiveRoom/);
  assert.match(html, /authorPerspectiveRoom:\s*compactAuthorPerspectiveRoom\(message\.authorPerspectiveRoom\)/);
  assert.doesNotMatch(html, /author-perspective[\s\S]{0,1400}saveLearningNote/);
  assert.doesNotMatch(html, /author-perspective[\s\S]{0,1400}data-dossier-source-decision/);
});

test("workflow intent actions reuse reversible UI steps without auto-promoting evidence", () => {
  assert.match(html, /handleWorkflowIntentAction/);
  assert.match(html, /focusFirstStructuredDataField/);
  assert.match(html, /fillQuestionFromWorkflowPrompt/);
  assert.match(html, /updateAuditFeedback\(messageIndex,\s*"retry"\)/);
  assert.doesNotMatch(html, /data-workflow-action="accept-evidence"/);
  assert.doesNotMatch(html, /data-workflow-action="save-experiment"/);
});

test("learning dossiers expose OpenHuman memory sync status", () => {
  assert.match(html, /renderOpenHumanMemoryStatus/);
  assert.match(html, /已沉淀到 OpenHuman 本地记忆/);
  assert.match(html, /沉淀到 OpenHuman 本地记忆失败/);
  assert.match(html, /语义索引未完成/);
  assert.match(html, /message\.openhumanMemory/);
  assert.match(html, /dossier\.openhumanMemory/);
  assert.match(html, /syncMessageMemoryFromDossiers/);
});

test("learning dossiers show saved validation pack progress", () => {
  assert.match(html, /renderDossierValidationPack/);
  assert.match(html, /验证任务包/);
  assert.match(html, /验证进度/);
  assert.match(html, /workbench\.validationPack/);
  assert.match(html, /data-dossier-validation-prompt/);
  assert.match(html, /任务包不是作者原文证据/);
});

test("learning dossiers show saved synthesis guide as a study product", () => {
  assert.match(html, /renderDossierSynthesisGuide/);
  assert.match(html, /综合讲义/);
  assert.match(html, /workbench\.synthesisGuide/);
  assert.match(html, /dossier\.synthesisAnswer/);
  assert.match(html, /data-dossier-synthesis-prompt/);
  assert.match(html, /系统综合不是作者原文证据/);
  assert.match(html, /lockDossierDetailActions[\s\S]{0,700}"\[data-dossier-synthesis-prompt\]"/);
  assert.doesNotMatch(html, /dossier-synthesis[\s\S]{0,900}data-dossier-source-decision/);
});

test("learning overview exposes top-level topic learning paths", () => {
  assert.match(html, /renderLearningPathOverview/);
  assert.match(html, /主题学习路径/);
  assert.match(html, /overview\.learningPaths/);
  assert.match(html, /data-learning-path-open/);
  assert.match(html, /不写入原始知识库/);
});

test("learning overview exposes an Amazon learning mastery panel", () => {
  assert.match(html, /renderMasteryPanel/);
  assert.match(html, /亚马逊学习掌握面板/);
  assert.match(html, /overview\.mastery/);
  assert.match(html, /renderMasteryStage/);
  assert.match(html, /renderMasteryTopic/);
  assert.match(html, /mastery\.stages/);
  assert.match(html, /mastery\.topics/);
  assert.match(html, /mastery-panel/);
  assert.match(html, /mastery-stage/);
  assert.match(html, /mastery-topic/);
  assert.match(html, /不写入原始知识库/);
  assert.match(html, /mastery\.boundary/);
});

test("learning overview exposes a source-first topic package", () => {
  assert.match(html, /renderSourcePackage/);
  assert.match(html, /主题来源包/);
  assert.match(html, /已采纳证据/);
  assert.match(html, /候选来源/);
  assert.match(html, /已排除来源/);
  assert.match(html, /基于已采纳证据继续学习/);
  assert.match(html, /打开档案确认来源/);
  assert.match(html, /打开原文并确认是否有用/);
  assert.match(html, /data-topic-evidence-dossier/);
  assert.match(html, /applySourcePackageControls/);
  assert.match(html, /data-source-package-excluded/);
  assert.match(html, /data-source-package-allowed/);
  assert.match(html, /data-source-package-selected/);
  assert.match(html, /候选来源未被采纳前/);
});

test("learning overview renders source-backed topic study guides", () => {
  assert.match(html, /renderStudyGuideProduct/);
  assert.match(html, /source-backed-claims/);
  assert.match(html, /作者视角/);
  assert.match(html, /执行清单/);
  assert.match(html, /复习追问/);
  assert.match(html, /基于讲义追问并回到原文核对/);
  assert.match(html, /data-learning-product-prompt/);
  assert.match(html, /data-learning-product-copy/);
  assert.match(html, /data-learning-product-download/);
  assert.match(html, /copyStudyHandout/);
  assert.match(html, /downloadStudyHandoutMarkdown/);
  assert.match(html, /主题学习讲义/);
  assert.match(html, /handoutMarkdown/);
  assert.match(html, /data-topic-source-focus/);
  assert.match(html, /focusTopicStudyGuideSource/);
  assert.match(html, /data-source-context-payload/);
  assert.match(html, /handleLearningProductPrompt/);
  assert.match(html, /只有已采纳原文证据/);
  assert.doesNotMatch(html, /source-backed-claims[\s\S]{0,1200}data-dossier-source-decision/);
});

test("learning overview renders auditable evidence reports with source drilldown", () => {
  assert.match(html, /renderEvidenceReportProduct/);
  assert.match(html, /if \(type === "evidence_report"\) return "报告";/);
  assert.match(html, /可审计学习报告/);
  assert.match(html, /claim-audit-list/);
  assert.match(html, /source-ledger-list/);
  assert.match(html, /data-evidence-report-source-focus/);
  assert.match(html, /基于报告追问并回到原文核对/);
  assert.match(html, /querySelectorAll\("\[data-evidence-report-source-focus\]"\)/);
  assert.match(html, /focusTopicStudyGuideSource/);
  assert.match(html, /data-source-context-payload/);
  assert.match(html, /不能替代原文/);
  assert.doesNotMatch(html, /evidence-report[\s\S]{0,1400}data-dossier-source-decision/);
});

test("topic source reading room keeps candidate sources explicit and user-selected", () => {
  assert.match(html, /topicReadingRoom/);
  assert.match(html, /renderTopicReadingRoom/);
  assert.match(html, /来源阅读室/);
  assert.match(html, /grid-template-rows:\s*auto auto auto minmax\(260px,\s*1fr\) auto/);
  assert.match(html, /min-height:\s*min\(420px,\s*55vh\)/);
  assert.match(html, /候选\/待确认来源/);
  assert.match(html, /data-topic-source-toggle/);
  assert.match(html, /先选择来源再继续学习/);
  assert.match(html, /基于已选来源继续核对：当前主题/);
  assert.match(html, /allowedSourceKeys/);
  assert.match(html, /selectedSources/);
  assert.match(html, /候选来源默认不会被采纳/);
  assert.match(html, /候选来源未确认前不是证据/);
  assert.doesNotMatch(html, /candidate[\s\S]{0,400}checked/);
});

test("topic source reading room uses selected source controls without auto-saving evidence", () => {
  assert.match(html, /continueTopicReading/);
  assert.match(html, /buildTopicReadingPrompt/);
  assert.match(html, /sourceKeysForTopicSource/);
  assert.match(html, /uniqueSourceKeysForTopicSources/);
  assert.match(html, /本次只使用我在来源阅读室选择的/);
  assert.doesNotMatch(html, /topic-reading[\s\S]{0,1000}saveLearningNote/);
  assert.doesNotMatch(html, /topic-reading[\s\S]{0,1000}data-dossier-source-decision/);
});

test("inline evidence references fall back to visible source evidence order", () => {
  assert.match(html, /fallbackSourceEvidenceClaimTarget/);
  assert.match(html, /source-evidence:/);
  assert.match(html, /data-claim-key\^=/);
  assert.match(html, /focusClaimByKey\(fallbackTarget\)/);
});

test("learning overview and dossier detail expose evidence adoption path", () => {
  assert.match(html, /data-learning-path-evidence/);
  assert.match(html, /data-evidence-needed-dossier/);
  assert.match(html, /openDossierSourceSection/);
  assert.match(html, /补来源证据/);
  assert.match(html, /renderEvidenceNeededProduct/);
  assert.match(html, /候选来源不会自动成为证据/);
  assert.match(html, /renderDossierSources/);
  assert.match(html, /data-dossier-source-decision/);
  assert.match(html, /确认这段有用/);
  assert.match(html, /saveDossierSourceDecision/);
  assert.match(html, /业务材料和实验结果不会当成作者原文证据/);
});

test("source-backed learning paths can jump to product material intake", () => {
  assert.match(html, /data-learning-path-materials/);
  assert.match(html, /补产品材料/);
  assert.match(html, /focusProductMaterialEntry/);
  assert.match(html, /产品\/ASIN/);
  assert.match(html, /主图现状/);
  assert.match(html, /CTR/);
  assert.match(html, /CVR/);
  assert.match(html, /核心关键词/);
  assert.match(html, /不是作者原文证据/);
});

test("material-backed learning paths can jump to validation and experiment review", () => {
  assert.match(html, /data-learning-path-validation/);
  assert.match(html, /看验证方案/);
  assert.match(html, /focusValidationPlanEntry/);
  assert.match(html, /实验名称/);
  assert.match(html, /CTR 前\/后/);
  assert.match(html, /CVR 前\/后/);
  assert.match(html, /ACOS 前\/后/);
  assert.match(html, /保存材料不代表已验证/);
});

test("fallback assistant messages preserve validation packs before saving dossiers", () => {
  assert.match(html, /validationPack:\s*payload\.validationPack/);
  assert.match(html, /validationPack:\s*compactValidationPack\(message\.validationPack\)/);
});

test("assistant answers keep learning memory separate from source citations", () => {
  assert.match(html, /renderLearningMemoryReminder/);
  assert.match(html, /renderLearningMemoryAlignment/);
  assert.match(html, /本地学习档案提醒/);
  assert.match(html, /memoryKindLabel/);
  assert.match(html, /历史业务材料/);
  assert.match(html, /历史实验复盘/);
  assert.match(html, /历史档案与本轮作者证据不一致/);
  assert.match(html, /不是作者原文证据/);
  assert.match(html, /message\.learningMemoryReminder/);
  assert.doesNotMatch(html, /learning-memory[\s\S]{0,800}data-source-target/);
});

test("mobile graph list keeps clickable source nodes visible", () => {
  assert.match(html, /prioritizeGraphMobileNodes/);
  assert.match(html, /source:\s*1/);
  assert.doesNotMatch(html, /\.filter\(\(node\) => \["question", "point", "step", "evidence", "concept", "source"\]\.includes\(node\.type\)\)\s*\.slice\(0,\s*12\)/);
});
