"""
施工合同纠纷 — 逐笔债权诉讼时效计算器

功能: 根据每笔债权的应付款日期, 自动计算3年诉讼时效届满日,
      支持时效中断事由输入, 输出逐笔时效状态分析。

用法:
    python3 scripts/calculate_limitation.py --input claims.json [--output report.md]

输入格式 (claims.json):
[
    {
        "id": "A1",
        "type": "进度款",
        "description": "第1期进度款",
        "due_date": "2020-07-10",
        "amount": 2000000.00,
        "interruptions": [
            {"date": "2022-06-15", "type": "催款函送达", "evidence": "快递单号SF123456"},
            {"date": "2023-01-20", "type": "部分付款", "evidence": "银行流水"}
        ]
    }
]

时效中断类型:
    - 催款函/律师函送达 (民法典第195条第1项)
    - 部分付款/承诺还款 (民法典第195条第2项)
    - 起诉/仲裁 (民法典第195条第3项)
    - 对账单/结算协议确认
"""

import json
import sys
from datetime import date, timedelta
from typing import Optional

# 诉讼时效期间 (民法典第188条)
LIMITATION_YEARS = 3


def calc_limitation_expiry(due_date: date, interruptions: list[dict]) -> dict:
    """
    计算单笔债权的时效届满日, 考虑时效中断。

    逻辑:
    1. 初始时效届满日 = 应付款日 + 3年
    2. 每个中断事由:
       a. 中断日必须在时效届满日前
       b. 从中断日起重新计算3年时效
    3. 返回最终时效届满日及状态

    Returns:
        {
            "original_expiry": date,       # 无中断时的时效届满日
            "current_expiry": date,        # 考虑中断后的最终时效届满日
            "is_expired": bool,            # 是否已届满
            "days_remaining": int,         # 剩余天数 (负数=已过期天数)
            "interruption_count": int,     # 有效中断次数
            "last_interruption": str|null  # 最后一次有效中断
        }
    """
    initial_expiry = due_date + timedelta(days=LIMITATION_YEARS * 365)

    current_expiry = initial_expiry
    effective_interruptions = 0
    last_interruption = None

    # 按日期排序中断事由
    sorted_interruptions = sorted(interruptions, key=lambda i: i["date"])

    for interruption in sorted_interruptions:
        int_date = date.fromisoformat(interruption["date"])

        # 中断必须发生在时效届满前
        if int_date <= current_expiry:
            current_expiry = int_date + timedelta(days=LIMITATION_YEARS * 365)
            effective_interruptions += 1
            last_interruption = interruption["type"]

    today = date.today()
    days_remaining = (current_expiry - today).days

    return {
        "original_expiry": initial_expiry.isoformat(),
        "current_expiry": current_expiry.isoformat(),
        "is_expired": days_remaining < 0,
        "days_remaining": days_remaining,
        "interruption_count": effective_interruptions,
        "last_interruption": last_interruption,
    }


def evaluate_status(result: dict) -> str:
    """根据剩余天数评估时效风险状态"""
    if result["is_expired"]:
        return "❌ 时效已届满"
    elif result["days_remaining"] <= 90:
        return "⚠️ 即将届满 (90天内)"
    elif result["days_remaining"] <= 365:
        return "⚠️ 时效风险 (1年内)"
    else:
        return "✅ 时效存续"


def analyze_claims(claims: list[dict]) -> dict:
    """
    逐笔分析债权时效, 生成汇总报告。

    Returns:
        {
            "claims": [...],                # 每笔债权的详细分析
            "summary": {...},              # 汇总统计
            "generated_at": str            # 生成时间
        }
    """
    results = []
    total_amount = 0.0
    expired_amount = 0.0
    risk_amount = 0.0
    safe_amount = 0.0
    expired_count = 0
    risk_count = 0
    safe_count = 0

    for claim in claims:
        due_date = date.fromisoformat(claim["due_date"])
        interruptions = claim.get("interruptions", [])
        limitation_result = calc_limitation_expiry(due_date, interruptions)
        status = evaluate_status(limitation_result)

        amount = claim.get("amount", 0)
        total_amount += amount

        if limitation_result["is_expired"]:
            expired_amount += amount
            expired_count += 1
        elif limitation_result["days_remaining"] <= 365:
            risk_amount += amount
            risk_count += 1
        else:
            safe_amount += amount
            safe_count += 1

        results.append({
            **claim,
            "limitation": limitation_result,
            "status": status,
        })

    return {
        "claims": results,
        "summary": {
            "total_claims": len(claims),
            "total_amount": round(total_amount, 2),
            "expired": {"count": expired_count, "amount": round(expired_amount, 2)},
            "at_risk": {"count": risk_count, "amount": round(risk_amount, 2)},
            "safe": {"count": safe_count, "amount": round(safe_amount, 2)},
        },
        "generated_at": date.today().isoformat(),
    }


def format_markdown_report(analysis: dict) -> str:
    """格式化输出 Markdown 报告"""
    lines = []

    lines.append("# 逐笔债权诉讼时效分析报告")
    lines.append(f"\n> 生成日期: {analysis['generated_at']}")
    lines.append(f"> 债权总数: {analysis['summary']['total_claims']} 笔")
    lines.append(f"> 债权总额: ¥{analysis['summary']['total_amount']:,.2f}\n")

    # 汇总表
    lines.append("## 时效状态汇总\n")
    lines.append("| 状态 | 笔数 | 金额 | 占比 |")
    lines.append("|------|------|------|------|")
    total = analysis["summary"]["total_amount"] or 1  # 避免除零
    s = analysis["summary"]
    lines.append(f"| ✅ 时效存续 | {s['safe']['count']} | ¥{s['safe']['amount']:,.2f} | {s['safe']['amount']/total*100:.1f}% |")
    lines.append(f"| ⚠️ 即将届满/有风险 | {s['at_risk']['count']} | ¥{s['at_risk']['amount']:,.2f} | {s['at_risk']['amount']/total*100:.1f}% |")
    lines.append(f"| ❌ 时效已届满 | {s['expired']['count']} | ¥{s['expired']['amount']:,.2f} | {s['expired']['amount']/total*100:.1f}% |")

    # 逐笔明细
    lines.append("\n## 逐笔时效分析\n")
    lines.append("| 编号 | 债权类型 | 应付日期 | 金额 | 时效届满日 | 剩余天数 | 中断次数 | 状态 |")
    lines.append("|------|---------|---------|------|-----------|---------|---------|------|")
    for claim in analysis["claims"]:
        lim = claim["limitation"]
        lines.append(
            f"| {claim['id']} | {claim['type']} | {claim['due_date']} "
            f"| ¥{claim['amount']:,.2f} | {lim['current_expiry']} "
            f"| {lim['days_remaining']}天 | {lim['interruption_count']}次 "
            f"| {claim['status']} |"
        )

    # 时效届满/即将届满专项
    expired_claims = [c for c in analysis["claims"] if c["limitation"]["is_expired"]]
    if expired_claims:
        lines.append("\n## ❌ 已届满债权 — 紧急处置建议\n")
        for c in expired_claims:
            lines.append(f"- **{c['id']} {c['description']}**：时效已于 {c['limitation']['current_expiry']} 届满")
            lines.append(f"  金额：¥{c['amount']:,.2f}")
            lines.append(f"  建议：立即排查是否存在未录入的时效中断事由（催款记录/部分付款/对账确认）。")
            lines.append(f"  如确实无中断事由，该笔债权已丧失胜诉权，仅可作为谈判筹码主张自然债务。\n")

    risk_claims = [c for c in analysis["claims"]
                   if not c["limitation"]["is_expired"] and c["limitation"]["days_remaining"] <= 365]
    if risk_claims:
        lines.append("\n## ⚠️ 即将届满债权 — 优先处置建议\n")
        for c in risk_claims:
            lines.append(f"- **{c['id']} {c['description']}**：时效将于 {c['limitation']['current_expiry']} 届满（剩余 {c['limitation']['days_remaining']} 天）")
            lines.append(f"  金额：¥{c['amount']:,.2f}")
            lines.append(f"  建议：立即发送催款函中断时效 + 准备起诉/仲裁材料\n")

    # 脚注
    lines.append("\n---")
    lines.append("*本报告为律师审查草稿。时效计算基于《民法典》第188-195条，3年诉讼时效。*")
    lines.append("*中断事由来自用户提供的记录，未经独立核实。实际时效状态以有效中断证据为准。*")

    return "\n".join(lines)


def main():
    if len(sys.argv) < 3 or sys.argv[1] != "--input":
        print("用法: python3 calculate_limitation.py --input claims.json [--output report.md]")
        print()
        print("示例 claims.json:")
        print(json.dumps([
            {
                "id": "A1",
                "type": "进度款",
                "description": "第1期进度款",
                "due_date": "2020-07-10",
                "amount": 2000000.00,
                "interruptions": [
                    {"date": "2022-06-15", "type": "催款函送达", "evidence": "快递单号SF123456"}
                ]
            }
        ], indent=2, ensure_ascii=False))
        sys.exit(1)

    input_file = sys.argv[2]
    output_file = None
    if len(sys.argv) >= 5 and sys.argv[3] == "--output":
        output_file = sys.argv[4]

    with open(input_file, "r", encoding="utf-8") as f:
        claims = json.load(f)

    analysis = analyze_claims(claims)
    report = format_markdown_report(analysis)

    if output_file:
        with open(output_file, "w", encoding="utf-8") as f:
            f.write(report)
        print(f"报告已写入: {output_file}")
    else:
        print(report)


if __name__ == "__main__":
    main()
