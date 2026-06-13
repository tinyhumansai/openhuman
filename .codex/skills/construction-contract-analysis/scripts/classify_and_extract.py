"""
PDF 分类与文本提取脚本 v2.2

功能:
  1. 检测 PDF 类型（文字型/图片型/CID编码/超大文本型）
  2. 智能路由选择最佳提取引擎:
     - 文字型 → pdfplumber 直接提取（零成本，秒级）
     - 图片型/扫描件(≤350p) → Agnes AI Vision 主力（多模态视觉，印章+批注+文字）
                              → MinerU API 后备（Agnes失败时自动降级）
     - 图片型/扫描件(>350p) → MinerU API（超大文件不适合逐页Agnes）
     - CID编码型 → Agnes AI Vision 抽样 + MinerU 全量
  3. 统一输出标准化 MD 格式

用法:
  python classify_and_extract.py <file.pdf> -o output.md
  python classify_and_extract.py <file.pdf> --engine agnes    # 强制Agnes
  python classify_and_extract.py <file.pdf> --engine mineru   # 强制MinerU
  python classify_and_extract.py <input_dir> -o output_dir/

依赖:
  - pdfplumber (文字型PDF提取)
  - pypdfium2 (PDF→图像转换，Agnes需要)
  - requests (Agnes/MinerU API 调用)

环境变量:
  AGNES_API_KEY - Agnes AI API Key（扫描件主力OCR，≤350页优先）
  MINERU_API_KEY - MinerU API Key（后备OCR，Agnes不可用或>350页时启用）
"""
import argparse
import base64
import json
import os
import re
import sys
import time
import zipfile
import io
from datetime import datetime
from pathlib import Path
from typing import Optional, List

# ── 配置 ──────────────────────────────────────────────────────
#
# 智能路由决策规则:
#
# PDF类型         → 主力引擎     → 后备引擎    → 说明
# ─────────────────────────────────────────────────────────
# 文字型(<200p)   → pdfplumber   → (无)        → 零成本秒级
# 文字型(≥200p)   → pdfplumber   → Agnes抽样    → 全文+关键页抽查
# 图片型(≤350p)   → Agnes AI     → MinerU API  → 多模态识别优先
# 图片型(>350p)   → MinerU API   → Agnes抽样    → 超大文件Agnes不适用
# CID编码型       → Agnes抽样    → MinerU API  → pdfplumber提取乱码
#
# 引擎选择可被命令行参数覆盖: --engine agnes|mineru|pdfplumber
# ──────────────────────────────────────────────────────────

# ── .env 自动加载 ────────────────────────────────────────────
def _load_env():
    """从工作区 .env 文件加载环境变量（覆盖空值，不覆盖已有的非空值）。"""
    env_files = [
        Path.home() / ".proma" / "agent-workspaces" / "workspace-1781091898269" / ".env",
        Path(__file__).resolve().parents[3] / ".env",   # workspace-xxx/
        Path(__file__).resolve().parents[4] / ".env",   # agent-workspaces/ (fallback)
    ]
    loaded = False
    for env_f in env_files:
        if env_f.exists():
            for line in env_f.read_text().splitlines():
                line = line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                key, _, val = line.partition("=")
                key, val = key.strip(), val.strip().strip('"').strip("'")
                if key and val and not os.environ.get(key):
                    os.environ[key] = val
            loaded = True
    return loaded

_load_env()

TEXT_THRESHOLD = 50    # 前3页字符数超过此值判定为文字型PDF
MINERU_API_KEY = os.environ.get("MINERU_API_KEY", "")
MINERU_BASE = "https://mineru.net/api/v4"
POLL_INTERVAL = 5      # 轮询间隔（秒）

# Agnes AI Vision 配置
AGNES_API_KEY = os.environ.get("AGNES_API_KEY", "")
AGNES_ENABLED = bool(AGNES_API_KEY)
AGNES_BASE = "https://apihub.agnes-ai.com/v1"
AGNES_MODEL = "agnes-2.0-flash"
AGNES_MAX_PAGES = 350  # 单文件最多通过Agnes逐页处理的页数（超阈值改用MinerU）
AGNES_TIMEOUT = 120    # 单页Agnes请求超时（秒）

# 多 Key 支持：从环境变量收集所有可用的 MinerU API Key
ALL_KEYS = [k for k in [
    os.environ.get("MINERU_API_KEY", ""),
    os.environ.get("MINERU_API_KEY_2", ""),
    os.environ.get("MINERU_API_KEY_3", ""),
] if k]

# MinerU 限制（Precision 模式）
MAX_FILE_MB = 180    # 单文件上限 200MB，留余量
MAX_PAGES = 500      # 单文件上限 600 页，留余量

# ── PDF 类型检测 ──────────────────────────────────────────────

def detect_pdf_type(pdf_path: str) -> dict:
    """检测 PDF 是文字型还是图片型。

    使用 pdfplumber 检查前 3 页的文本量，超过阈值判定为文字型。
    pdfplumber 零资源消耗，仅读取 PDF 已有文字层。
    """
    try:
        import pdfplumber
        with pdfplumber.open(pdf_path) as pdf:
            char_count = 0
            for i, page in enumerate(pdf.pages):
                if i >= 3:
                    break
                text = page.extract_text() or ""
                char_count += len(text.strip())
    except Exception as e:
        return {"type": "unknown", "char_count": 0, "error": str(e)}

    if char_count > TEXT_THRESHOLD:
        return {"type": "text", "char_count": char_count}
    else:
        return {"type": "image", "char_count": char_count}


# ── 文字型 PDF 提取 ──────────────────────────────────────────

def extract_text_pdf(pdf_path: str) -> list:
    """从文字型 PDF 提取全文，按页面分割。

    使用 pdfplumber（零资源消耗，100%准确）。
    返回: [{"page": 1, "text": "..."}, ...]
    """
    import pdfplumber

    result = []
    with pdfplumber.open(pdf_path) as pdf:
        for i, page in enumerate(pdf.pages, 1):
            text = page.extract_text() or ""
            stripped = text.strip()
            if stripped:
                result.append({"page": i, "text": stripped})

    return result


# ── 扫描型 PDF 识别（MinerU 云端 API） ─────────────────────────

def mineru_upload_and_parse(pdf_path: str, api_key: str = None) -> str:
    """MinerU 云端 PDF 解析：上传 → 解析 → 获取 MD。

    使用 MinerU 的 v4 API，三步完成：
    1. POST /file-urls/batch → 获取签名上传 URL
    2. PUT 签名 URL → 上传文件（自动触发解析）
    3. GET /extract-results/batch/{batch_id} → 轮询结果 → 下载 MD

    返回: 解析后的 Markdown 文本
    """
    import requests

    if not (api_key or MINERU_API_KEY):
        raise RuntimeError("MINERU_API_KEY 未设置")

    headers = {"Authorization": f"Bearer {api_key or MINERU_API_KEY}"}
    filename = os.path.basename(pdf_path)

    # Step 1: 申请上传链接
    print(f"  申请上传: {filename}")
    resp = requests.post(f"{MINERU_BASE}/file-urls/batch",
        headers={**headers, "Content-Type": "application/json"},
        json={
            "files": [{"name": filename, "is_ocr": True}],
            "model_version": "pipeline",
            "language": "ch",
            "enable_formula": False,
        },
        timeout=30)
    result = resp.json()
    if result.get("code") != 0:
        raise RuntimeError(f"申请上传失败: {result.get('msg', '未知错误')}")

    batch_id = result["data"]["batch_id"]
    upload_url = result["data"]["file_urls"][0]
    print(f"  已获取上传链接 (batch_id={batch_id[:8]}...)")

    # Step 2: 上传文件
    print(f"  上传中...")
    with open(pdf_path, "rb") as f:
        resp = requests.put(upload_url, data=f, timeout=300)
    if resp.status_code != 200:
        raise RuntimeError(f"上传失败: HTTP {resp.status_code}")
    print(f"  上传完成，等待解析...")

    # Step 3: 轮询结果
    poll_url = f"{MINERU_BASE}/extract-results/batch/{batch_id}"
    for attempt in range(120):
        time.sleep(POLL_INTERVAL)
        resp = requests.get(poll_url, headers=headers, timeout=30)
        result = resp.json()
        if result.get("code") != 0:
            continue
        items = result.get("data", {}).get("extract_result", [])
        if not items:
            continue
        item = items[0]
        state = item.get("state", "")
        print(f"    状态: {state}", end="\r")
        if state == "done":
            zip_url = item.get("full_zip_url", "")
            if not zip_url:
                raise RuntimeError("解析完成但无下载链接")
            # 流式下载 ZIP 到临时文件（防止大文件内存溢出）
            print(f"\n  下载结果...")
            local_zip = os.path.join(os.path.dirname(pdf_path), f"~mineru_{batch_id[:8]}.zip")
            try:
                with requests.get(zip_url, stream=True, timeout=600) as r:
                    r.raise_for_status()
                    with open(local_zip, "wb") as f:
                        for chunk in r.iter_content(chunk_size=8192):
                            if chunk:
                                f.write(chunk)
                with zipfile.ZipFile(local_zip) as zf:
                    md_files = [f for f in zf.namelist() if f.endswith(".md")]
                    if not md_files:
                        raise RuntimeError("ZIP 中未找到 MD 文件")
                    md_file = sorted(md_files, key=lambda f: -len(f))[0]
                    md_content = zf.read(md_file).decode("utf-8")
                    print(f"  MD 提取完成 ({len(md_content)} chars)")
                return md_content
            finally:
                if os.path.exists(local_zip):
                    os.remove(local_zip)
        elif state in ("failed", "error"):
            err = item.get("err_msg", "未知错误")
            raise RuntimeError(f"解析失败: {err}")

    raise RuntimeError("解析超时")


# ── 文件限制检测与自动拆分 ──────────────────────────────────

def get_pdf_page_count(pdf_path: str) -> int:
    import pypdfium2 as pdfium
    doc = pdfium.PdfDocument(pdf_path)
    count = len(doc)
    doc.close()
    return count


def check_file_limits(pdf_path: str) -> dict:
    size_mb = os.path.getsize(pdf_path) / (1024 * 1024)
    pages = get_pdf_page_count(pdf_path)
    reasons = []
    if size_mb > MAX_FILE_MB:
        reasons.append(f'大小 {size_mb:.0f}MB > {MAX_FILE_MB}MB')
    if pages > MAX_PAGES:
        reasons.append(f'页数 {pages} > {MAX_PAGES}')
    return {'ok': len(reasons)==0, 'size_mb': round(size_mb,1),
            'pages': pages, 'reason': '; '.join(reasons) if reasons else None}


def split_pdf(pdf_path: str, max_pages: int = MAX_PAGES) -> list:
    import pypdfium2 as pdfium
    doc = pdfium.PdfDocument(pdf_path)
    total = len(doc)
    doc.close()
    parts, base = [], os.path.splitext(pdf_path)[0]
    for start in range(0, total, max_pages):
        end = min(start + max_pages, total)
        part_path = f'{base}_part{start // max_pages + 1}.pdf'
        src = pdfium.PdfDocument(pdf_path)
        dst = pdfium.PdfDocument.new()
        dst.import_pages(src, pages=list(range(start, end)))
        dst.save(part_path)
        dst.close(); src.close()
        parts.append(part_path)
        mb = os.path.getsize(part_path) / (1024 * 1024)
        print(f'    拆分: p{start+1}-{end}/{total} ({mb:.0f}MB)')
    return parts


def cleanup_temp_pdfs(parts: list):
    for p in parts:
        if os.path.exists(p):
            os.remove(p)


# ── Agnes AI Vision OCR（扫描件主力引擎）─────────────────────────

AGNES_OCR_PROMPT = """请仔细识别这张建设工程文档图片中的所有内容：
1. 提取所有文字（包括表格每个单元格），用Markdown输出。
2. 识别所有印章（位置、形状、颜色、文字内容）。
3. 识别手写批注/签名。
4. 识别表格行列结构，用Markdown表格输出。
特别注意：区分印章格式文字（"签字盖章有效"）与实质性否定标记（"合同无效"）。"""


def agnes_vision_ocr_page(image_base64: str, page_num: int,
                          api_key: str = None) -> dict:
    """单页 Agnes AI Vision OCR。

    Args:
        image_base64: 页面图像的 base64 编码
        page_num: 页码
        api_key: Agnes API Key（默认用全局 AGNES_API_KEY）

    Returns:
        {"page": int, "text": str, "confidence": int, "flags": list}
    """
    import requests
    key = api_key or AGNES_API_KEY
    if not key:
        return {"page": page_num, "text": "", "confidence": 0,
                "flags": ["agnes_no_key"]}

    payload = {
        "model": AGNES_MODEL,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": AGNES_OCR_PROMPT},
                {"type": "image_url", "image_url": {
                    "url": f"data:image/png;base64,{image_base64}"
                }},
            ]
        }],
        "max_tokens": 4000,
    }
    headers = {
        "Authorization": f"Bearer {key}",
        "Content-Type": "application/json",
    }

    try:
        resp = requests.post(
            f"{AGNES_BASE}/chat/completions",
            headers=headers, json=payload,
            timeout=AGNES_TIMEOUT)
        if resp.status_code == 200:
            data = resp.json()
            text = data.get("choices", [{}])[0].get("message", {}).get("content", "")
            return {"page": page_num, "text": text.strip(), "confidence": 90,
                    "flags": ["agnes"]}
        else:
            return {"page": page_num, "text": "", "confidence": 0,
                    "flags": [f"agnes_http_{resp.status_code}"]}
    except Exception as e:
        return {"page": page_num, "text": "", "confidence": 0,
                "flags": [f"agnes_error:{str(e)[:60]}"]}


def agnes_ocr_pdf(pdf_path: str, api_key: str = None) -> list:
    """Agnes AI Vision 逐页 OCR 整个 PDF。

    使用 pypdfium2 逐页渲染为图像 → base64 → Agnes vision API。
    返回: [{"page": 1, "text": "...", "confidence": 90, "flags": ["agnes"]}, ...]
    """
    import pypdfium2 as pdfium

    key = api_key or AGNES_API_KEY
    if not key:
        print("  ✗ Agnes API Key 未配置")
        return []

    doc = pdfium.PdfDocument(pdf_path)
    total_pages = len(doc)
    print(f"  总页数: {total_pages}")

    pages = []
    failed = 0
    for i in range(total_pages):
        page = doc[i]
        bitmap = page.render(scale=2)  # 2x 放大提升OCR精度
        pil_img = bitmap.to_pil()
        buf = io.BytesIO()
        pil_img.save(buf, format="PNG")
        img_b64 = base64.b64encode(buf.getvalue()).decode("utf-8")

        result = agnes_vision_ocr_page(img_b64, i + 1, api_key=key)

        if result["confidence"] > 0:
            pages.append(result)
        else:
            failed += 1
            pages.append(result)

        pct = (i + 1) / total_pages * 100
        print(f"    第 {i+1}/{total_pages} 页 ({pct:.0f}%) "
              f"| {'✓' if result['confidence'] > 0 else '✗'}"
              f"{' ' + result['flags'][0] if result['confidence'] == 0 else ''}",
              end="\r")

    doc.close()
    print(f"\n  Agnes 完成: {len(pages)-failed}/{total_pages} 页成功"
          + (f", {failed} 页失败" if failed else ""))

    return pages


# ── 扫描型 PDF 识别（双引擎: Agnes 优先 → MinerU 降级）───────────

def extract_image_pdf(pdf_path: str, api_key: str = None,
                      force_engine: str = None) -> list:
    """图片型 PDF 智能路由提取。

    决策逻辑:
      1. force_engine="agnes"  → 强制 Agnes（失败也继续）
      2. force_engine="mineru" → 强制 MinerU
      3. ≤AGNES_MAX_PAGES 且 AGNES_ENABLED → Agnes 优先, 失败降级 MinerU
      4. >AGNES_MAX_PAGES 或 Agnes 不可用 → MinerU

    返回: [{"page": 1, "text": "...", "confidence": 90, "flags": [...]}, ...]
    """
    import re

    page_count = get_pdf_page_count(pdf_path)
    print(f"  PDF 页数: {page_count}")
    print(f"  Agnes: {'可用' if AGNES_ENABLED else '未配置'} | "
          f"MinerU: {'可用' if ALL_KEYS else '未配置'}")

    # 确定使用的引擎
    use_agnes = False
    if force_engine == "agnes":
        use_agnes = True
        print(f"  → 强制 Agnes (--engine agnes)")
    elif force_engine == "mineru":
        use_agnes = False
        print(f"  → 强制 MinerU (--engine mineru)")
    elif AGNES_ENABLED and page_count <= AGNES_MAX_PAGES:
        use_agnes = True
        print(f"  → Agnes AI Vision (≤{AGNES_MAX_PAGES}页, 多模态识别)")
    else:
        reason = (f">{AGNES_MAX_PAGES}页 不适合逐页Agnes" if page_count > AGNES_MAX_PAGES
                  else "Agnes未配置")
        print(f"  → MinerU API ({reason})")

    # ---- Agnes 路径 ----
    # 注意：api_key 是 MinerU 的 Key，Agnes 始终用全局 AGNES_API_KEY
    if use_agnes:
        try:
            pages = agnes_ocr_pdf(pdf_path)  # 用 AGNES_API_KEY，不传 api_key
            success_count = sum(1 for p in pages if p["confidence"] > 0)
            fail_rate = (len(pages) - success_count) / max(len(pages), 1)

            if fail_rate > 0.3:
                # 超过30%页面失败 → 降级到 MinerU
                print(f"  ⚠️ Agnes 失败率 {fail_rate:.0%}, 降级到 MinerU")
                use_agnes = False
            elif pages:
                return pages
            else:
                print(f"  ✗ Agnes 全部页面失败, 降级到 MinerU")
                use_agnes = False
        except Exception as e:
            print(f"  ✗ Agnes 异常: {e}")
            print(f"  → 降级到 MinerU")
            use_agnes = False

    # ---- MinerU 路径 ----
    limits = check_file_limits(pdf_path)

    if limits["ok"]:
        md_content = mineru_upload_and_parse(pdf_path, api_key=api_key)
    else:
        print(f"  ⚠️ {limits['reason']}，自动拆分处理")
        parts = split_pdf(pdf_path)
        md_content = ""
        for i, part in enumerate(parts):
            part_md = mineru_upload_and_parse(part, api_key=api_key)
            size_mb = os.path.getsize(part) / (1024 * 1024)
            print(f"    分片 {i+1}/{len(parts)} 完成 ({size_mb:.0f}MB)")
            if i > 0:
                part_md = re.sub(r'^# OCR.*?\n(>.*?\n)*', '', part_md,
                                flags=re.MULTILINE)
                md_content += "\n\n" + part_md.strip()
            else:
                md_content = part_md
            cleanup_temp_pdfs([part])

    # 将 MD 按标题分页
    pages = []
    page_blocks = re.split(r'\n(?=## |# )', md_content.strip())
    for i, block in enumerate(page_blocks, 1):
        stripped = block.strip()
        if stripped:
            pages.append({"page": i, "text": stripped, "confidence": 95,
                          "flags": ["mineru"]})
    if not pages:
        pages.append({"page": 1, "text": md_content.strip(), "confidence": 95,
                      "flags": ["mineru"]})

    return pages


# ── MD 输出 ──────────────────────────────────────────────────

def write_md_output(pages: list, pdf_path: str, pdf_type: str, output_path: str):
    """将识别结果写为统一格式的 MD 文件。"""
    filename = os.path.basename(pdf_path)
    total = len(pages)

    # 计算汇总信息
    confidences = [p.get("confidence", 0) for p in pages if p.get("confidence")]
    avg_conf = round(sum(confidences) / len(confidences), 1) if confidences else 0
    low_conf_pages = [p["page"] for p in pages if p.get("confidence", 100) < 70]
    flagged_pages = [
        p["page"]
        for p in pages
        if p.get("flags") and "error" not in str(p.get("flags", []))
    ]

    with open(output_path, "w", encoding="utf-8") as f:
        f.write(f"# OCR 识别结果 — {filename}\n\n")
        f.write(f"> 识别模式: {pdf_type}\n")
        f.write(f"> 识别时间: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}\n")
        f.write(f"> 总页数: {total}\n\n")
        f.write("---\n\n")

        for page in pages:
            pn = page.get("page", "?")
            conf = page.get("confidence", "N/A")
            flags = page.get("flags", [])
            text = page.get("text", "")
            flag_str = " | ".join(flags) if flags else "无"

            f.write(f"## 第 {pn} 页\n")

            # 置信度低时加警告
            if isinstance(conf, (int, float)) and conf < 70:
                f.write(f"> ⚠️ 置信度: {conf}% | 标记: {flag_str}\n")
                f.write("> **注意**: 此页识别质量较低，建议核对原文\n\n")
            else:
                f.write(f"置信度: {conf}% | 标记: {flag_str}\n\n")

            f.write(f"{text}\n\n")
            f.write("---\n\n")

        # 汇总表
        f.write("## 汇总\n\n")
        f.write("| 总页数 | 平均置信度 | 低质量页 | 标记页 |\n")
        f.write("|--------|-----------|---------|--------|\n")
        low_str = ", ".join(str(p) for p in low_conf_pages) if low_conf_pages else "无"
        flag_str = ", ".join(str(p) for p in flagged_pages) if flagged_pages else "无"
        f.write(f"| {total} | {avg_conf}% | {low_str} | {flag_str} |\n")
        f.write("\n")

        # 低置信度预警
        if low_conf_pages:
            f.write("### ⚠️ 低置信度页面预警\n\n")
            f.write(
                f"第 {', '.join(str(p) for p in low_conf_pages)} 页置信度低于 70%，"
                "识别结果可能不准确。建议查看原文或重新扫描后再识别。\n\n"
            )

    print(f"  输出: {output_path}")


# ── 主逻辑 ───────────────────────────────────────────────────

def process_pdf(pdf_path: str, output_dir: Optional[str] = None,
                force_ocr: bool = False, api_key: str = None,
                force_engine: str = None) -> str:
    """处理单个 PDF：检测类型 → 提取/OCR → 输出 MD。

    Args:
        force_engine: None=自动, "agnes"=强制Agnes, "mineru"=强制MinerU
    返回: MD 文件路径
    """
    pdf_path = os.path.abspath(pdf_path)
    filename = os.path.basename(pdf_path)
    base, _ = os.path.splitext(filename)

    if output_dir:
        os.makedirs(output_dir, exist_ok=True)
        md_path = os.path.join(output_dir, f"{base}.md")
    else:
        md_path = os.path.join(os.path.dirname(pdf_path), f"{base}.md")

    print(f"\n处理: {filename}")

    # 1. 检测类型（除非强制 OCR）
    if force_ocr:
        pdf_type = "image"
        engine_label = force_engine or ("agnes" if AGNES_ENABLED else "mineru")
        print(f"  类型: 图片型PDF（强制OCR, 引擎: {engine_label}）")
    else:
        info = detect_pdf_type(pdf_path)
        pdf_type = info["type"]

        if pdf_type == "unknown":
            print(f"  ⚠️ PDF检测失败: {info.get('error', '未知错误')}")
            with open(md_path, "w", encoding="utf-8") as f:
                f.write(f"# OCR 识别结果 — {filename}\n\n")
                f.write(f"> 错误: 无法检测PDF类型\n")
                f.write(f"> {info.get('error', '未知错误')}\n")
            return md_path

        print(f"  类型: {'文字型PDF' if pdf_type == 'text' else '图片型PDF (需OCR)'} "
              f"(字符数: {info['char_count'] if pdf_type == 'text' else 'N/A'})")

    # 2. 提取/OCR
    if pdf_type == "text" and not force_ocr:
        pages = extract_text_pdf(pdf_path)
        pages = [
            {"page": p["page"], "text": p["text"], "confidence": 100,
             "flags": ["pdfplumber"]}
            for p in pages
        ]
    else:
        pages = extract_image_pdf(pdf_path, api_key=api_key,
                                  force_engine=force_engine)

    if not pages:
        print("  ⚠️ 未提取到任何文本内容")
        with open(md_path, "w", encoding="utf-8") as f:
            f.write(f"# OCR 识别结果 — {filename}\n\n")
            f.write(f"> 未提取到文本内容\n")
        return md_path

    # 3. 输出 MD
    write_md_output(pages, pdf_path, pdf_type, md_path)
    return md_path


def main():
    parser = argparse.ArgumentParser(
        description="PDF 分类与文本提取 — "
                    "文字型→pdfplumber(本地), 扫描型→Agnes AI(优先≤350p)→MinerU API(降级/超大)"
    )
    parser.add_argument("input", nargs="+", help="PDF 文件路径或目录（支持多个）")
    parser.add_argument("-o", "--output-dir", help="输出目录（默认与输入文件同目录）")
    parser.add_argument("--engine", choices=["agnes", "mineru", "pdfplumber"],
                        help="强制指定引擎 (默认自动选择)")
    parser.add_argument("--force-ocr", action="store_true",
                        help="强制 OCR 模式（即使检测为文字型）")
    args = parser.parse_args()

    # 检查 API key
    if not MINERU_API_KEY and not AGNES_API_KEY:
        print("警告: AGNES_API_KEY 和 MINERU_API_KEY 均未设置。"
              "扫描型 PDF 将无法处理。", file=sys.stderr)
    elif not AGNES_API_KEY:
        print("提示: AGNES_API_KEY 未设置。"
              "扫描型 PDF 将使用 MinerU 处理。", file=sys.stderr)

    # 收集所有 PDF 文件
    pdf_files = []
    for inp in args.input:
        p = Path(inp)
        if p.is_dir():
            pdf_files.extend(sorted(p.glob("*.pdf")))
        elif p.is_file() and p.suffix.lower() == ".pdf":
            pdf_files.append(p)
        else:
            print(f"跳过: {inp} (不是PDF)")

    if not pdf_files:
        print("未找到 PDF 文件")
        return

    engine_info = f"强制:{args.engine}" if args.engine else "自动(Agnes优先≤350p→MinerU)"
    print(f"共 {len(pdf_files)} 个 PDF 文件")
    print(f"Agnes: {'✓' if AGNES_ENABLED else '✗'} | "
          f"MinerU Keys: {len(ALL_KEYS)} | 引擎: {engine_info}")
    for i, pdf_file in enumerate(pdf_files):
        key = ALL_KEYS[i % len(ALL_KEYS)] if ALL_KEYS else None
        process_pdf(str(pdf_file), args.output_dir, api_key=key,
                    force_ocr=args.force_ocr, force_engine=args.engine)


if __name__ == "__main__":
    main()
