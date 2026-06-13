"""
施工合同纠纷分析 — 共享工具函数
LPR 利率查询 / 金额格式化 / 日期解析 / 利息计算辅助
"""

import re
from datetime import date, datetime, timedelta
from decimal import Decimal, ROUND_HALF_UP
from typing import Optional, Union

# ── LPR 历史利率表 ─────────────────────────────────────────────
# 数据来源: 全国银行间同业拆借中心 (www.chinamoney.com.cn)
# 格式: (生效日期, 1年期LPR, 5年期以上LPR)
# 注意: LPR 自 2019-08-20 起发布, 此前按同期贷款基准利率

LPR_TABLE = [
    # (生效日期,      1Y LPR,  5Y+ LPR)
    ("2024-11-20",    3.10,   3.60),
    ("2024-10-21",    3.10,   3.60),
    ("2024-09-20",    3.35,   3.85),
    ("2024-08-20",    3.35,   3.85),
    ("2024-07-22",    3.35,   3.85),
    ("2024-06-20",    3.45,   3.95),
    ("2024-05-20",    3.45,   3.95),
    ("2024-04-22",    3.45,   3.95),
    ("2024-03-20",    3.45,   3.95),
    ("2024-02-20",    3.45,   3.95),
    ("2024-01-22",    3.45,   4.20),
    ("2023-12-20",    3.45,   4.20),
    ("2023-11-20",    3.45,   4.20),
    ("2023-10-20",    3.45,   4.20),
    ("2023-09-20",    3.45,   4.20),
    ("2023-08-21",    3.45,   4.20),
    ("2023-07-20",    3.55,   4.20),
    ("2023-06-20",    3.55,   4.20),
    ("2023-05-22",    3.65,   4.30),
    ("2023-04-20",    3.65,   4.30),
    ("2023-03-20",    3.65,   4.30),
    ("2023-02-20",    3.65,   4.30),
    ("2023-01-20",    3.65,   4.30),
    ("2022-12-20",    3.65,   4.30),
    ("2022-11-21",    3.65,   4.30),
    ("2022-10-20",    3.65,   4.30),
    ("2022-09-20",    3.65,   4.30),
    ("2022-08-22",    3.65,   4.30),
    ("2022-07-20",    3.70,   4.45),
    ("2022-06-20",    3.70,   4.45),
    ("2022-05-20",    3.70,   4.45),
    ("2022-04-20",    3.70,   4.60),
    ("2022-03-21",    3.70,   4.60),
    ("2022-02-21",    3.70,   4.60),
    ("2022-01-20",    3.70,   4.60),
    ("2021-12-20",    3.80,   4.65),
    ("2021-11-22",    3.85,   4.65),
    ("2021-10-20",    3.85,   4.65),
    ("2021-09-22",    3.85,   4.65),
    ("2021-08-20",    3.85,   4.65),
    ("2021-07-20",    3.85,   4.65),
    ("2021-06-21",    3.85,   4.65),
    ("2021-05-20",    3.85,   4.65),
    ("2021-04-20",    3.85,   4.65),
    ("2021-03-22",    3.85,   4.65),
    ("2021-02-20",    3.85,   4.65),
    ("2021-01-20",    3.85,   4.65),
    ("2020-12-21",    3.85,   4.65),
    ("2020-11-20",    3.85,   4.65),
    ("2020-10-20",    3.85,   4.65),
    ("2020-09-21",    3.85,   4.65),
    ("2020-08-20",    3.85,   4.65),
    ("2020-07-20",    3.85,   4.65),
    ("2020-06-22",    3.85,   4.65),
    ("2020-05-20",    3.85,   4.65),
    ("2020-04-20",    3.85,   4.65),
    ("2020-03-20",    4.05,   4.75),
    ("2020-02-20",    4.05,   4.75),
    ("2020-01-20",    4.15,   4.80),
    ("2019-12-20",    4.15,   4.80),
    ("2019-11-20",    4.15,   4.80),
    ("2019-10-21",    4.20,   4.85),
    ("2019-09-20",    4.20,   4.85),
    ("2019-08-20",    4.25,   4.85),
]

# 2019-08-19 之前的贷款基准利率 (1年以内)
LOAN_BASE_RATES = [
    ("2015-10-24", 4.35),
    ("2015-08-26", 4.60),
    ("2015-06-28", 4.85),
    ("2015-05-11", 5.10),
    ("2015-03-01", 5.35),
    ("2014-11-22", 5.60),
]


def get_lpr(target_date: date, term: str = "1Y") -> Optional[float]:
    """
    查询指定日期的 LPR 利率。

    Args:
        target_date: 查询日期
        term: "1Y" (1年期, 默认) 或 "5Y" (5年期以上)

    Returns:
        LPR 利率 (%), 未找到返回 None
    """
    idx = 1 if term == "5Y" else 0
    for eff_date_str, lpr_1y, lpr_5y in LPR_TABLE:
        eff_date = date.fromisoformat(eff_date_str)
        if target_date >= eff_date:
            return lpr_5y if idx else lpr_1y
    return None


def get_loan_base_rate(target_date: date) -> Optional[float]:
    """查询 2019-08-19 之前的贷款基准利率 (1年以内)"""
    for eff_date_str, rate in LOAN_BASE_RATES:
        eff_date = date.fromisoformat(eff_date_str)
        if target_date >= eff_date:
            return rate
    return None


def get_interest_rate(target_date: date, term: str = "1Y") -> float:
    """
    获取适用的逾期利息计算利率。
    2019-08-20 后返回同期 LPR, 此前返回贷款基准利率。
    如未找到, 返回默认 LPR (最新一期 1Y LPR)。
    """
    lpr_cutoff = date(2019, 8, 20)
    if target_date >= lpr_cutoff:
        rate = get_lpr(target_date, term)
        if rate is not None:
            return rate
    rate = get_loan_base_rate(target_date)
    if rate is not None:
        return rate
    # 兜底: 返回最新 1Y LPR
    return 3.10


def get_lpr_segments(
    start_date: date, end_date: date, term: str = "1Y"
) -> list[dict]:
    """
    将时间段按 LPR 变更点分段, 返回每段的起止日期和适用利率。

    Returns:
        [{ "start": date, "end": date, "rate": float, "days": int }, ...]
    """
    idx = 1 if term == "5Y" else 0
    segments = []
    current = start_date

    # 获取 start_date 到 end_date 之间的所有 LPR 变更点
    change_dates = []
    for eff_date_str, _, _ in LPR_TABLE:
        d = date.fromisoformat(eff_date_str)
        if start_date < d <= end_date:
            change_dates.append(d)
    change_dates.sort()

    for change in change_dates:
        rate = get_lpr(current, term) or get_interest_rate(current, term)
        days = (change - current).days
        if days > 0:
            segments.append({
                "start": current,
                "end": change,
                "rate": rate,
                "days": days,
            })
        current = change

    # 最后一段
    rate = get_lpr(current, term) or get_interest_rate(current, term)
    final_days = (end_date - current).days
    if final_days >= 0:
        segments.append({
            "start": current,
            "end": end_date,
            "rate": rate,
            "days": final_days,
        })

    return segments


# ── 金额格式化 ──────────────────────────────────────────────────

def format_cny(amount: Union[float, Decimal], decimal_places: int = 2) -> str:
    """格式化为人民币大写金额样式, 如 ¥1,234,567.89"""
    if isinstance(amount, Decimal):
        d = amount
    else:
        d = Decimal(str(amount))
    d = d.quantize(Decimal("0." + "0" * decimal_places), rounding=ROUND_HALF_UP)

    # 构造 ¥x,xxx.xx 格式
    parts = f"{d:,.{decimal_places}f}"
    return f"¥{parts}"


def cny_to_chinese(amount: Union[float, Decimal]) -> str:
    """将金额转换为中文大写 (用于法律文书), 如 1234567.89 → 壹佰贰拾叁万肆仟伍佰陆拾柒元捌角玖分"""
    if isinstance(amount, Decimal):
        num = amount
    else:
        num = Decimal(str(amount))
    num = num.quantize(Decimal("0.00"), rounding=ROUND_HALF_UP)

    digit_cn = ["零", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖"]
    radices_cn = ["", "拾", "佰", "仟", "万", "拾", "佰", "仟", "亿"]
    tail_cn = ["元", "角", "分"]

    integer_part = int(num)
    decimal_part = int((num - integer_part) * 100)

    result = ""
    if integer_part == 0:
        result = "零元"
    else:
        digits = []
        while integer_part > 0:
            digits.append(integer_part % 10)
            integer_part //= 10

        need_zero = False
        for i in range(len(digits) - 1, -1, -1):
            d = digits[i]
            if d == 0:
                need_zero = True
                # 万位和亿位处理
                if i in (4, 8):
                    result += radices_cn[i]
                    need_zero = False
            else:
                if need_zero:
                    result += "零"
                    need_zero = False
                result += digit_cn[d] + radices_cn[i]
        result += "元"

    jiao = decimal_part // 10
    fen = decimal_part % 10
    if jiao == 0 and fen == 0:
        result += "整"
    elif jiao == 0:
        result += "零" + digit_cn[fen] + "分"
    elif fen == 0:
        result += digit_cn[jiao] + "角整"
    else:
        result += digit_cn[jiao] + "角" + digit_cn[fen] + "分"

    return result


# ── 日期解析 ────────────────────────────────────────────────────

_CHINESE_DATE_RE = re.compile(
    r"(\d{4})\s*年\s*(\d{1,2})\s*月\s*(\d{1,2})\s*日"
)
_ISO_DATE_RE = re.compile(r"(\d{4})-(\d{2})-(\d{2})")
_DOT_DATE_RE = re.compile(r"(\d{4})\.(\d{1,2})\.(\d{1,2})")


def parse_date(text: str) -> Optional[date]:
    """
    解析多种格式日期字符串: 2024年3月15日 / 2024-03-15 / 2024.3.15 / 20240315
    """
    text = text.strip()
    m = _CHINESE_DATE_RE.search(text)
    if m:
        return date(int(m.group(1)), int(m.group(2)), int(m.group(3)))
    m = _ISO_DATE_RE.search(text)
    if m:
        return date(int(m.group(1)), int(m.group(2)), int(m.group(3)))
    m = _DOT_DATE_RE.search(text)
    if m:
        return date(int(m.group(1)), int(m.group(2)), int(m.group(3)))
    # 8位数字: 20240315
    if len(text) == 8 and text.isdigit():
        return date(int(text[:4]), int(text[4:6]), int(text[6:8]))
    return None


def format_date_cn(d: date) -> str:
    """格式化为中文日期: 2024年3月15日"""
    return f"{d.year}年{d.month}月{d.day}日"


# ── 利息计算辅助 ────────────────────────────────────────────────

def calc_interest(
    principal: float,
    start_date: date,
    end_date: date,
    term: str = "1Y",
    rate_override: Optional[float] = None,
) -> dict:
    """
    分段计算逾期利息。

    Args:
        principal: 本金 (元)
        start_date: 计息起始日
        end_date: 计息终止日 (不含当日利息)
        term: "1Y" 或 "5Y"
        rate_override: 固定年利率 (%), 不按 LPR 分段

    Returns:
        {
            "principal": float,       # 本金
            "total_interest": float,  # 总利息
            "total_days": int,        # 总天数
            "segments": [...]         # 各段明细
        }
    """
    if rate_override is not None:
        days = (end_date - start_date).days
        interest = principal * (rate_override / 100) * (days / 365)
        return {
            "principal": principal,
            "total_interest": round(interest, 2),
            "total_days": days,
            "annual_rate": rate_override,
            "segments": [{
                "start": start_date.isoformat(),
                "end": end_date.isoformat(),
                "days": days,
                "rate": rate_override,
                "interest": round(interest, 2),
            }],
        }

    segments = get_lpr_segments(start_date, end_date, term)
    total_interest = 0.0
    detail_segments = []
    total_days = 0

    for seg in segments:
        seg_interest = principal * (seg["rate"] / 100) * (seg["days"] / 365)
        seg_interest = round(seg_interest, 2)
        total_interest += seg_interest
        total_days += seg["days"]
        detail_segments.append({
            "start": seg["start"].isoformat(),
            "end": seg["end"].isoformat(),
            "days": seg["days"],
            "rate": seg["rate"],
            "interest": seg_interest,
        })

    return {
        "principal": principal,
        "total_interest": round(total_interest, 2),
        "total_days": total_days,
        "segments": detail_segments,
    }


def calc_interest_with_payments(
    principal: float,
    start_date: date,
    payments: list[dict],
    term: str = "1Y",
) -> dict:
    """
    考虑部分还款的利息计算 (先抵利息后抵本金 — 司法清偿顺序)。

    payments: [{ "date": date, "amount": float }, ...]

    Returns:
        {
            "principal": float,          # 初始本金
            "remaining_principal": float, # 剩余未还本金
            "total_interest": float,      # 应付总利息
            "total_paid_interest": float, # 已付利息
            "total_paid_principal": float,# 已还本金
            "unpaid_interest": float,     # 未付利息
            "entries": [...]             # 逐期明细
        }
    """
    sorted_payments = sorted(payments, key=lambda p: p["date"])
    remaining = principal
    current_start = start_date
    total_interest = 0.0
    total_paid_interest = 0.0
    total_paid_principal = 0.0
    entries = []

    for payment in sorted_payments:
        pay_date = payment["date"]
        pay_amount = payment["amount"]

        # 计算从 current_start 到 pay_date 的利息
        result = calc_interest(remaining, current_start, pay_date, term)
        accrued_interest = result["total_interest"]
        total_interest += accrued_interest

        # 清偿顺序: 先利息后本金
        if pay_amount <= accrued_interest:
            # 全部冲抵利息
            paid_interest = pay_amount
            paid_principal = 0
            unpaid_interest_for_period = accrued_interest - pay_amount
        else:
            paid_interest = accrued_interest
            paid_principal = pay_amount - accrued_interest
            unpaid_interest_for_period = 0

        total_paid_interest += paid_interest
        total_paid_principal += paid_principal
        remaining -= paid_principal

        entries.append({
            "period_start": current_start.isoformat(),
            "period_end": pay_date.isoformat(),
            "principal_before": round(remaining + paid_principal, 2),
            "accrued_interest": round(accrued_interest, 2),
            "payment_amount": pay_amount,
            "paid_interest": round(paid_interest, 2),
            "paid_principal": round(paid_principal, 2),
            "unpaid_interest_carried": round(unpaid_interest_for_period, 2),
            "principal_after": round(remaining, 2),
        })

        current_start = pay_date

    return {
        "principal": principal,
        "remaining_principal": round(remaining, 2),
        "total_interest": round(total_interest, 2),
        "total_paid_interest": round(total_paid_interest, 2),
        "total_paid_principal": round(total_paid_principal, 2),
        "unpaid_interest": round(total_interest - total_paid_interest, 2),
        "entries": entries,
    }
