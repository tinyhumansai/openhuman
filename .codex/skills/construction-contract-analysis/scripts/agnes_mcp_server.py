#!/usr/bin/env python3
"""Agnes AI MCP Server — 文本对话 + 多模态视觉OCR

工具:
  - agnes_chat:       文本对话 (agnes-2.0-flash, 256K ctx)
  - agnes_vision_ocr: PDF/图片视觉OCR (印章/批注/表格识别)
  - agnes_list_models: 列出可用模型

注册: mcp.json → Proma 自动拉起
"""

import base64
import io
import json
import os
import sys
from pathlib import Path

import requests
from pypdfium2 import PdfDocument
from mcp.server.fastmcp import FastMCP

def _load_api_key():
    """尝试多来源加载 API Key: 环境变量 → .env 文件"""
    key = os.environ.get("AGNES_API_KEY", "")
    if key:
        return key
    # 从工作区 .env 文件读取（依次尝试多个路径）
    env_files = [
        Path.home() / ".proma" / "agent-workspaces" / "workspace-1781091898269" / ".env",
        Path(__file__).resolve().parents[3] / ".env",   # workspace-xxx/
        Path(__file__).resolve().parents[4] / ".env",   # agent-workspaces/ (fallback)
    ]
    for env_f in env_files:
        if env_f.exists():
            for line in env_f.read_text().splitlines():
                line = line.strip()
                if line.startswith("AGNES_API_KEY="):
                    return line.split("=", 1)[1].strip().strip('"').strip("'")
    return ""

API_KEY = _load_api_key()
BASE_URL = "https://apihub.agnes-ai.com/v1"
MODEL = "agnes-2.0-flash"
TIMEOUT = 180

mcp = FastMCP("agnes-ai")


def _chat(messages, max_tokens=4000, temperature=0.7, thinking=False):
    payload = {"model": MODEL, "messages": messages,
               "max_tokens": max_tokens, "temperature": temperature}
    if thinking:
        payload["chat_template_kwargs"] = {"enable_thinking": True}
    headers = {"Authorization": f"Bearer {API_KEY}", "Content-Type": "application/json"}
    resp = requests.post(f"{BASE_URL}/chat/completions", headers=headers,
                         json=payload, timeout=TIMEOUT)
    return resp.json()


def _vision_ocr(image_base64, prompt=None):
    if not prompt:
        prompt = """请仔细识别这张建设工程文档图片中的所有内容：
1. 提取所有文字（包括表格每个单元格），用Markdown输出。
2. 识别所有印章（位置、形状、颜色、文字内容）。
3. 识别手写批注/签名。
4. 识别表格行列结构，用Markdown表格输出。
特别注意：区分印章格式文字（"签字盖章有效"）与实质性否定标记（"合同无效"）。"""
    return _chat([{"role": "user", "content": [
        {"type": "text", "text": prompt},
        {"type": "image_url", "image_url": {"url": f"data:image/jpeg;base64,{image_base64}"}}
    ]}], max_tokens=4000)


def _models():
    headers = {"Authorization": f"Bearer {API_KEY}"}
    resp = requests.get(f"{BASE_URL}/models", headers=headers, timeout=30)
    return [m["id"] for m in resp.json().get("data", [])]


@mcp.tool()
def agnes_chat(messages: list, max_tokens: int = 4000,
               temperature: float = 0.7, thinking: bool = False) -> str:
    """调用 Agnes AI (agnes-2.0-flash) 进行文本对话。256K上下文，支持thinking推理模式。适合法律分析、合同审查、文书起草。

    Args:
        messages: OpenAI格式消息列表 [{"role":"user","content":"..."}]
        max_tokens: 最大生成token数
        temperature: 温度参数(0-2)
        thinking: 是否启用深度推理模式
    """
    result = _chat(messages, max_tokens, temperature, thinking)
    return result.get("choices", [{}])[0].get("message", {}).get("content",
                    json.dumps(result, ensure_ascii=False))


@mcp.tool()
def agnes_list_models() -> str:
    """列出当前可用Agnes AI模型及定价信息"""
    models = _models()
    return f"Agnes AI 可用模型: {', '.join(models)}\n当前使用: {MODEL}\n定价: 输入 $0/1M tokens, 输出 $0/1M tokens (免费)"


@mcp.tool()
def agnes_vision_ocr(pdf_path: str, page_num: int = None,
                     prompt: str = None) -> str:
    """用 Agnes 多模态视觉对PDF/图片进行OCR。支持印章检测、批注识别、表格结构还原。扫描件主力OCR引擎。

    Args:
        pdf_path: PDF文件的绝对路径
        page_num: 单页识别页码(0-indexed，不传则处理全部页)
        prompt: 自定义OCR提示词（可选）
    """
    p = Path(pdf_path)
    if not p.exists():
        return f"错误: PDF不存在 — {pdf_path}"

    pdf = PdfDocument(str(p))
    total = len(pdf)
    pages = [page_num] if page_num is not None else list(range(total))

    output = [f"# Agnes Vision OCR\n模型: {MODEL} | 文件: {p.name} | 页数: {len(pages)}/{total}\n"]

    for pg in pages:
        output.append(f"\n## 第 {pg+1} 页\n")
        try:
            page = pdf[pg]
            bitmap = page.render(scale=2.0)
            img = bitmap.to_pil()
            buf = io.BytesIO()
            img.save(buf, format='JPEG', quality=80)
            b64 = base64.b64encode(buf.getvalue()).decode()

            result = _vision_ocr(b64, prompt)
            if 'choices' in result:
                output.append(result['choices'][0]['message']['content'])
            else:
                output.append(f"失败: {result.get('error', 'Unknown')}")
        except Exception as e:
            output.append(f"异常: {e}")

    return '\n'.join(output)


if __name__ == "__main__":
    if not API_KEY:
        print("FATAL: AGNES_API_KEY not set", file=sys.stderr)
        sys.exit(1)
    import asyncio
    asyncio.run(mcp.run_stdio_async())
