"""
文本型 PDF 快速条款提取（绕过 OCR）
用于有文本层的 PDF，快速识别 23 类施工合同条款

用法:
  python extract_clauses.py contract.pdf [--output clauses.json]
"""

import argparse
import json
import re
import sys
from pathlib import Path

try:
    import pdfplumber
except ImportError:
    print("需要 pdfplumber 包: pip3 install pdfplumber")
    sys.exit(1)


# ── 23 类条款关键词 ─────────────────────────────────────────────

CLAUSE_PATTERNS = [
    # (编号, 名称, 关键词列表)
    ("C1", "造价模式", [
        "计价方式", "合同价款", "工程价款", "总价", "单价",
        "包干", "暂列金", "暂估价", "可调价", "成本加酬金",
        "固定总价", "固定单价", "合同价格",
    ]),
    ("C2", "工程质量", [
        "质量标准", "质量要求", "验收标准", "隐蔽工程",
        "竣工验收", "质量合格", "工程质量", "分部分项验收",
    ]),
    ("C3", "工期约定", [
        "开工日期", "竣工日期", "合同工期", "总工期",
        "工期顺延", "节点工期", "工期延误", "停工", "复工",
    ]),
    ("C4", "违约责任", [
        "违约责任", "违约金", "逾期付款", "逾期竣工",
        "违约赔偿", "单方解除", "合同解除",
    ]),
    ("C5", "价款调价", [
        "价格调整", "材料调价", "人工调价", "政策调价",
        "价格波动", "市场价格", "调价公式", "价差",
    ]),
    ("C6", "不可抗力", [
        "不可抗力", "自然灾害", "疫情", "政府行为",
        "意外事件", "免责", "免责条款",
    ]),
    ("C7", "争议管辖", [
        "争议解决", "管辖", "仲裁", "诉讼",
        "协商", "调解", "法院管辖",
    ]),
    ("C8", "缺陷责任", [
        "缺陷责任", "保修", "质保", "质量保证",
        "质保金", "保留金", "缺陷修复", "质量保证金",
    ]),
    ("C9", "价款支付", [
        "进度款", "付款方式", "付款节点", "付款条件", "付款比例",
        "报审", "开票", "审批", "支付方式", "预付款",
        "承兑汇票", "供应链金融", "逾期利率", "逾期付款利息",
        "质保金返还", "竣工结算款", "付款周期",
    ]),
    ("C10", "变更签证", [
        "工程变更", "设计变更", "签证", "洽商",
        "增量", "追加", "变更程序", "变更价款",
    ]),
    ("C11", "转包分包", [
        "转包", "分包", "挂靠", "资质",
        "违法分包", "转包禁止", "指定分包",
    ]),
    ("C12", "合同解除", [
        "解除合同", "终止合同", "解除条件", "终止条件",
        "退场", "停工待命",
    ]),
    ("C13", "送达默示", [
        "送达", "通知", "催告", "签收",
        "默示", "书面形式", "通讯地址", "法律文书",
    ]),
    ("C14", "背靠背付款", [
        "背靠背", "收到业主付款", "业主付款后", "随业主付款",
        "按业主付款比例",
    ]),
    ("C15", "EPC总承包", [
        "EPC", "工程总承包", "设计施工总承包", "DB总承包", "交钥匙",
        "业主需求", "性能考核", "设计责任",
    ]),
    ("C16", "联合体承包", [
        "联合体", "共同投标", "主办方", "牵头方",
        "连带责任", "联合体协议", "成员分工",
    ]),
    ("C17", "甲指分包", [
        "指定分包", "甲指分包", "指定供应商", "发包人指定",
        "指定分包商",
    ]),
    ("C18", "垫资", [
        "垫资", "垫付", "垫付工程款", "先期投入",
        "先行垫付", "垫资利息", "资金占用费",
    ]),
    ("C19", "履约担保", [
        "履约担保", "履约保函", "支付担保", "见索即付",
        "履约保证金", "保函", "担保金额",
    ]),
    ("C20", "农民工工资", [
        "农民工", "工资保障", "实名制", "工资专户",
        "工资保证金", "总包代发", "保障农民工工资",
    ]),
    ("C21", "保险", [
        "工程一切险", "第三方责任险", "人身意外险", "安装工程一切险",
        "保险", "投保", "免赔额", "保险金额",
    ]),
    ("C22", "甲供材", [
        "甲供材", "甲供", "甲控材", "甲控", "发包人供应",
        "发包人提供材料", "甲指乙供", "供应材料", "材料供应计划",
        "损耗率", "超领", "领料单",
    ]),
    ("C23", "竣工验收", [
        "竣工验收", "竣工验收合格", "竣工报验", "竣工申请",
        "移交", "现场移交", "竣工日期", "甩项验收",
        "擅自使用", "竣工验收备案", "验收报告",
    ]),
]


def find_clause_section(text: str, keywords: list[str], window: int = 500) -> str:
    """在合同文本中搜索包含关键词的段落"""
    text_lower = text.lower()
    best_pos = -1

    for kw in keywords:
        pos = text_lower.find(kw.lower())
        if pos != -1 and (best_pos == -1 or pos < best_pos):
            best_pos = pos

    if best_pos == -1:
        return ""

    start = max(0, best_pos - window // 4)
    end = min(len(text), best_pos + window)
    # 扩展到段落边界
    while start > 0 and text[start] != "\n":
        start -= 1
    while end < len(text) and text[end] != "\n":
        end += 1

    return text[start:end].strip()


def extract_all_text(pdf_path: str) -> str:
    """提取 PDF 全文"""
    full_text = ""
    with pdfplumber.open(pdf_path) as pdf:
        for page in pdf.pages:
            text = page.extract_text()
            if text:
                full_text += text + "\n"
    return full_text


def extract_clauses(text: str, clause_patterns: list = None) -> list[dict]:
    """从合同全文提取各条款段落"""
    if clause_patterns is None:
        clause_patterns = CLAUSE_PATTERNS

    results = []
    for code, name, keywords in clause_patterns:
        section = find_clause_section(text, keywords)
        results.append({
            "code": code,
            "name": name,
            "found": bool(section),
            "keywords_matched": [kw for kw in keywords if kw.lower() in text.lower()],
            "section_text": section[:2000] if section else "",  # 限制长度
        })

    return results


def main():
    parser = argparse.ArgumentParser(description="文本型 PDF 条款快速提取")
    parser.add_argument("pdf_path", help="PDF 文件路径")
    parser.add_argument("--output", "-o", help="输出 JSON 文件路径")
    args = parser.parse_args()

    print(f"提取: {args.pdf_path}")
    full_text = extract_all_text(args.pdf_path)
    print(f"  总字符数: {len(full_text)}")

    clauses = extract_clauses(full_text)
    found = sum(1 for c in clauses if c["found"])
    print(f"  识别条款: {found}/{len(clauses)}")

    output = {
        "file": Path(args.pdf_path).name,
        "total_chars": len(full_text),
        "clauses_found": found,
        "clauses_total": len(clauses),
        "clauses": clauses,
    }

    if args.output:
        with open(args.output, "w", encoding="utf-8") as f:
            json.dump(output, f, ensure_ascii=False, indent=2)
        print(f"输出: {args.output}")
    else:
        print(json.dumps(output, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
