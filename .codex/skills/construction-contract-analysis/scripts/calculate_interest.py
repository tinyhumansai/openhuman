"""
LPR 分段计息计算器
接受 JSON 输入，输出分段计息结果

用法:
  python calculate_interest.py --principal 4850000 --start 2024-03-21 --end 2024-08-15
  python calculate_interest.py --input payment_data.json --output interest_result.json

输入 JSON 格式:
{
  "principal": 4850000,
  "start_date": "2024-03-21",
  "end_date": "2024-08-15",
  "payments": [
    {"date": "2024-06-01", "amount": 1000000}
  ],
  "rate_override": null,
  "term": "1Y"
}
"""

import argparse
import json
import sys
from datetime import date
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from utils import calc_interest, calc_interest_with_payments, format_cny


def main():
    parser = argparse.ArgumentParser(description="LPR 分段计息计算器")
    parser.add_argument("--input", "-i", help="输入 JSON 文件")
    parser.add_argument("--output", "-o", help="输出 JSON 文件")
    parser.add_argument("--principal", type=float, help="本金 (元)")
    parser.add_argument("--start", help="计息起始日 (YYYY-MM-DD)")
    parser.add_argument("--end", help="计息终止日 (YYYY-MM-DD)")
    parser.add_argument("--rate", type=float, help="固定年利率(pct) — 不按 LPR 分段")
    parser.add_argument("--term", default="1Y", choices=["1Y", "5Y"])
    args = parser.parse_args()

    if args.input:
        with open(args.input, "r", encoding="utf-8") as f:
            data = json.load(f)

        principal = data["principal"]
        start_date = date.fromisoformat(data["start_date"])
        end_date = date.fromisoformat(data.get("end_date", date.today().isoformat()))
        payments_raw = data.get("payments", [])
        payments = [
            {"date": date.fromisoformat(p["date"]), "amount": p["amount"]}
            for p in payments_raw
        ]
        rate_override = data.get("rate_override")

        if payments:
            result = calc_interest_with_payments(
                principal, start_date, payments, term=args.term
            )
        else:
            result = calc_interest(
                principal, start_date, end_date,
                term=args.term, rate_override=rate_override,
            )
    else:
        if not all([args.principal, args.start, args.end]):
            parser.error("需要 --principal --start --end 或 --input")
        principal = args.principal
        start_date = date.fromisoformat(args.start)
        end_date = date.fromisoformat(args.end)
        result = calc_interest(
            principal, start_date, end_date,
            term=args.term, rate_override=args.rate,
        )

    # 增加格式化输出
    result["principal_formatted"] = format_cny(result["principal"])
    if "total_interest" in result:
        result["total_interest_formatted"] = format_cny(result["total_interest"])

    if args.output:
        with open(args.output, "w", encoding="utf-8") as f:
            json.dump(result, f, ensure_ascii=False, indent=2)
        print(f"结果已保存到: {args.output}")

    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
