# 模块八：标准化成果输出

## 触发条件

用户提到：输出报告/台账/证据目录/质证意见/诉状/答辩状/律师函/法律意见书

## 输出物清单

| 输出物 | 格式 | 生成方式 | 受众 |
|--------|------|---------|------|
| 合同风险分析报告 | **Markdown** | Agent 按模板直接生成 | 客户/法务/管理层 |
| 工程款利息核算台账 | **.xlsx** | `generate_ledger.py` → xlsx Skill | 财务/司法鉴定 |
| 证据目录 | **.xlsx** | Agent 整理 → xlsx Skill | 律师/法院 |
| 争议焦点汇总表 | Markdown | Agent 直接输出 | 律师/法务 |
| 法律检索报告 | Markdown | `construction-legal-research` | 律师/法务 |
| 鉴定评估报告 | Markdown | `construction-expert-evaluation` | 律师/鉴定人 |
| 法律文书（起诉状/答辩状/代理词等） | Markdown | `construction-legal-writing` | 律师/法院 |
| 需求确认书 | Markdown | `requirement-grill` | 内部 |
| 案件交接文档 | Markdown | `case-handoff` | 内部 |

## 输出格式铁律

1. **.xlsx 仅限两类**：工程款利息核算台账、证据目录。其余全部用 Markdown
2. **Markdown 统一规范**：`#` 一级标题 → `##` 二级 → `###` 三级，表格用 GFM 格式
3. **来源标签不可省略**：`[法条原文]` `[司法解释]` `[裁判规则]` `[需验证]`
4. **关键判断标记**：主观结论必须标 `[需审查]`
5. **免责声明**：所有交付物尾部附律师审查草稿声明
6. **五组分离**：证据列举/质证意见/证据认定/查明事实/争议焦点不得混合编排

## 报告结构模板

详见 `references/output-templates.md`，包含 9 类交付物的完整内部结构。

## 生成策略

| 复杂度 | 做法 | 说明 |
|--------|------|------|
| 简单分析 | Agent 直接 Markdown 输出 | 节省 token，快速响应 |
| 正式报告 | 按 `output-templates.md` 逐节填充 | 结构完整，可读性强 |
| 核算台账 | 数据 → `generate_ledger.py` → .xlsx | Sheet 1 台账 + Sheet 2 分段计息 |
| 证据目录 | Agent 整理 → .xlsx | 五组分离，不含论证 |
